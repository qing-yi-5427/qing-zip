#include <windows.h>
#include <objidl.h>
#include <propidl.h>
#include <propvarutil.h>
#include <oleauto.h>
#include "Common/MyInitGuid.h" // Exactly one TU defines the official interface IIDs.
#include "7zip/Archive/IArchive.h"
#include "7zip/IStream.h"
#include "7zip/ICoder.h"
#include "7zip/PropID.h"
#include "bridge_contract.h"
#include <algorithm>
#include <atomic>
#include <cstring>
#include <limits>
#include <memory>
#include <mutex>
#include <string>
#include <string_view>
#include <utility>
#include <vector>

static_assert(sizeof(QzSelectedTime) == 16);
static_assert(sizeof(QzSelectedItem) == 96 && offsetof(QzSelectedItem, ctime) == 48);
static_assert(sizeof(QzSelectedVolume) == 24);
static_assert(sizeof(QzSelectedHost) == 88 && offsetof(QzSelectedHost, trace) == 80);
static_assert(sizeof(QzSelectedRequest) == 40);
static_assert(sizeof(QzSelectedReceipt) == 240 && offsetof(QzSelectedReceipt, reason) == 48);

namespace {
constexpr uint32_t NONE = UINT32_MAX;
constexpr uint32_t MAX_VOLUMES = 1024, MAX_ITEMS = 100000;
constexpr uint64_t MAX_MANIFEST_UNITS = 8ull * 1024 * 1024;
struct Failure { HRESULT hr; const char *reason; };
struct UnsupportedLayout { const char *reason; };
[[noreturn]] void fail(const char *reason, HRESULT hr = E_FAIL) { throw Failure{hr, reason}; }
void exact(HRESULT hr, const char *reason) { if (hr != S_OK) fail(reason, hr); }
template<class F> HRESULT boundary(F &&f) noexcept {
  try { return f(); } catch (const Failure &e) { return FAILED(e.hr) ? e.hr : E_FAIL; }
  catch (const std::bad_alloc &) { return E_OUTOFMEMORY; } catch (...) { return E_FAIL; }
}
template<class T> class Ref {
  T *p_ = nullptr;
public:
  Ref() = default;
  explicit Ref(T *p) : p_(p) {}
  ~Ref() { if (p_) p_->Release(); }
  Ref(const Ref &) = delete; Ref &operator=(const Ref &) = delete;
  T *get() const { return p_; }
  T *operator->() const { return p_; }
  T **put() { if (p_) p_->Release(); p_ = nullptr; return &p_; }
};
struct Variant {
  PROPVARIANT v{};
  Variant() { PropVariantInit(&v); }
  ~Variant() { PropVariantClear(&v); }
  Variant(const Variant &) = delete;
};
uint64_t number(const PROPVARIANT &v, bool absent = false) {
  if (v.vt == VT_UI8) return v.uhVal.QuadPart;
  if (v.vt == VT_UI4) return v.ulVal;
  if (v.vt == VT_EMPTY && absent) return 0;
  fail("unexpected numeric property type");
}
bool boolean(const PROPVARIANT &v, bool absent = true) {
  if (v.vt == VT_EMPTY && absent) return false;
  if (v.vt != VT_BOOL || (v.boolVal != VARIANT_TRUE && v.boolVal != VARIANT_FALSE))
    fail("unexpected boolean property type");
  return v.boolVal == VARIANT_TRUE;
}
std::wstring text(const PROPVARIANT &v, bool absent = false) {
  if (v.vt == VT_EMPTY && absent) return {};
  if (v.vt != VT_BSTR) fail("unexpected string property type");
  const UINT n = SysStringLen(v.bstrVal);
  if (n > 32767 || (n && !v.bstrVal)) fail("oversized string property");
  std::wstring s(v.bstrVal ? v.bstrVal : L"", n);
  if (s.find(L'\0') != std::wstring::npos) fail("embedded NUL in property");
  return s;
}
QzSelectedTime time_value(const PROPVARIANT &v) {
  QzSelectedTime t{};
  if (v.vt == VT_EMPTY) return t;
  if (v.vt != VT_FILETIME) fail("unexpected time property type");
  t.present = 1;
  t.filetime_100ns = (uint64_t(v.filetime.dwHighDateTime) << 32) | v.filetime.dwLowDateTime;
  t.precision = v.wReserved1;
  // Old handlers reserve garbage here when precision is not8/9 fractional digits.
  t.sub100ns = (v.wReserved1 == 24 || v.wReserved1 == 25 || v.wReserved1 == 3)
      ? v.wReserved2 : 0;
  if (t.sub100ns >= 100) fail("invalid sub-FILETIME precision");
  return t;
}
struct Item { std::wstring name; QzSelectedItem value{}; };

struct OperationStatus {
  const QzSelectedHost host; // Snapshot function table; context lifetime is host-owned.
  std::atomic<HRESULT> first_error{S_OK};
  explicit OperationStatus(const QzSelectedHost &h) : host(h) {}
  HRESULT finish(HRESULT hr) noexcept {
    if (hr != S_OK) {
      HRESULT expected = S_OK;
      first_error.compare_exchange_strong(expected, FAILED(hr) ? hr : E_FAIL);
    }
    return hr == S_OK ? S_OK : FAILED(hr) ? hr : E_FAIL;
  }
  HRESULT check() noexcept {
    if (host.cancelled(host.context)) finish(E_ABORT);
    return first_error.load();
  }
  template<class F> HRESULT call(F &&f) noexcept { return finish(boundary(std::forward<F>(f))); }
  HRESULT observe(uint32_t event, uint32_t index = UINT32_MAX, int32_t mode = 0,
                  int32_t result = 0, uint64_t value = 0) noexcept {
    return finish(host.trace(host.context, event, index, mode, result, value));
  }
  // QueryInterface(E_NOINTERFACE) is ordinary capability probing, not an I/O
  // error. The sole other non-error exception is the predetermined volume EOF.
};

struct Shared {
  const std::shared_ptr<OperationStatus> operation;
  const QzSelectedHost &host;
  std::vector<Item> items;
  std::vector<uint8_t> selected, gotten, prepared, done;
  std::vector<uint64_t> bytes;
  std::mutex ledger;
  std::atomic<uint64_t> decoded{0}, selected_bytes{0}, total{0};
  std::atomic<uint32_t> item_errors{0}, block_errors{0}, streams_open{0};
  uint32_t current = NONE;
  explicit Shared(std::shared_ptr<OperationStatus> s) : operation(std::move(s)), host(operation->host) {}
  HRESULT check() const { return operation->check(); }
  void record(HRESULT hr) noexcept { operation->finish(hr); }
  void count(uint32_t index, uint32_t n) {
    std::lock_guard<std::mutex> lock(ledger);
    if (index >= bytes.size() || bytes[index] > items[index].value.size ||
        n > items[index].value.size - bytes[index]) fail("output length exceeded");
    bytes[index] += n;
    decoded.fetch_add(n);
    if (selected[index]) selected_bytes.fetch_add(n);
  }
};

class VolumeStream final : public IInStream {
  std::atomic<ULONG> refs_{1};
  const std::shared_ptr<OperationStatus> operation_;
  const QzSelectedHost &host_;
  uint32_t index_; uint64_t length_, position_ = 0;
  std::mutex cursor_;
public:
  VolumeStream(std::shared_ptr<OperationStatus> s, uint32_t i, uint64_t n)
      : operation_(std::move(s)), host_(operation_->host), index_(i), length_(n) {}
  HRESULT STDMETHODCALLTYPE QueryInterface(REFIID iid, void **p) noexcept override {
    if (!p) return E_POINTER; *p = nullptr;
    if (iid == IID_IUnknown || iid == IID_ISequentialInStream || iid == IID_IInStream)
      *p = static_cast<IInStream *>(this);
    else return E_NOINTERFACE;
    AddRef(); return S_OK;
  }
  ULONG STDMETHODCALLTYPE AddRef() noexcept override { return ++refs_; }
  ULONG STDMETHODCALLTYPE Release() noexcept override { auto n = --refs_; if (!n) delete this; return n; }
  HRESULT STDMETHODCALLTYPE Read(void *data, UInt32 size, UInt32 *processed) noexcept override {
    if (processed) *processed = 0;
    return operation_->call([&]() -> HRESULT {
      exact(operation_->check(), "source operation failed or cancelled");
      if (size && !data) return E_POINTER;
      std::lock_guard<std::mutex> lock(cursor_);
      if (!size || position_ >= length_) return S_OK;
      uint32_t want = uint32_t(std::min<uint64_t>(size, length_ - position_)), got = 0;
      HRESULT hr = host_.read_volume_at(host_.context, index_, position_, data, want, &got);
      exact(operation_->observe(10, index_, 0, hr, got), "trace failure");
      if (got > want) return E_FAIL;
      position_ += got; if (processed) *processed = got;
      if (hr == S_OK && !got) return E_FAIL; // Premature EOF within captured length.
      return hr;
    });
  }
  HRESULT STDMETHODCALLTYPE Seek(Int64 offset, UInt32 origin, UInt64 *position) noexcept override {
    return operation_->call([&]() -> HRESULT {
      exact(operation_->check(), "source operation failed or cancelled");
      std::lock_guard<std::mutex> lock(cursor_);
      uint64_t base = origin == FILE_BEGIN ? 0 : origin == FILE_CURRENT ? position_ : length_;
      if (origin > FILE_END) return STG_E_INVALIDFUNCTION;
      if (offset < 0) {
        uint64_t distance = uint64_t(-(offset + 1)) + 1;
        if (distance > base) return HRESULT_FROM_WIN32(ERROR_NEGATIVE_SEEK);
        position_ = base - distance;
      } else {
        if (uint64_t(offset) > UINT64_MAX - base) return STG_E_INVALIDFUNCTION;
        position_ = base + uint64_t(offset);
      }
      if (position) *position = position_;
      exact(operation_->observe(11, index_, int32_t(origin), 0, position_), "trace failure");
      return S_OK;
    });
  }
};

std::wstring numbered(const std::wstring &prefix, uint32_t n) {
  std::wstring digits = std::to_wstring(n);
  if (digits.size() < 3) digits.insert(0, 3 - digits.size(), L'0');
  return prefix + digits;
}
struct VolumeMap {
  const std::shared_ptr<OperationStatus> operation;
  std::vector<std::wstring> names;
  std::vector<uint64_t> sizes;
  std::wstring terminal;
  uint64_t total = 0;
  VolumeMap(const QzSelectedRequest &r, std::shared_ptr<OperationStatus> s) : operation(std::move(s)) {
    if (!r.volumes || !r.volume_count || r.volume_count > MAX_VOLUMES)
      fail("invalid captured volume count");
    const auto &v0 = r.volumes[0];
    if (!v0.approved_leaf || !v0.leaf_units || v0.leaf_units > 32767)
      fail("invalid first volume name");
    std::wstring first(v0.approved_leaf, v0.leaf_units);
    if (first.size() <= 7 || CompareStringOrdinal(first.data() + first.size() - 7, 7, L".7z.001", 7, TRUE) != CSTR_EQUAL)
      fail("noncanonical first volume name");
    std::wstring prefix = first.substr(0, first.size() - 3);
    for (uint32_t i = 0; i < r.volume_count; ++i) {
      const auto &v = r.volumes[i];
      if (!v.approved_leaf || !v.leaf_units || v.leaf_units > 32767 || !v.fixed_size ||
          v.fixed_size > UINT64_MAX - total) fail("invalid captured volume");
      std::wstring leaf(v.approved_leaf, v.leaf_units);
      if (leaf.find_first_of(L"\\/:\0", 0, 4) != std::wstring::npos ||
          leaf != numbered(prefix, i + 1)) fail("volume names are not canonical contiguous leaves");
      names.push_back(leaf); sizes.push_back(v.fixed_size); total += v.fixed_size;
    }
    terminal = numbered(prefix, r.volume_count + 1);
  }
};

class OpenCallback final : public IArchiveOpenCallback, public IArchiveOpenVolumeCallback {
  std::atomic<ULONG> refs_{1};
  const VolumeMap &map_; bool volume_requests_;
public:
  OpenCallback(const VolumeMap &m, bool allow) : map_(m), volume_requests_(allow) {}
  HRESULT STDMETHODCALLTYPE QueryInterface(REFIID iid, void **p) noexcept override {
    if (!p) return E_POINTER; *p = nullptr;
    if (iid == IID_IUnknown || iid == IID_IArchiveOpenCallback)
      *p = static_cast<IArchiveOpenCallback *>(this);
    else if (iid == IID_IArchiveOpenVolumeCallback)
      *p = static_cast<IArchiveOpenVolumeCallback *>(this);
    else return E_NOINTERFACE; // No password support in initial unencrypted subset.
    AddRef(); return S_OK;
  }
  ULONG STDMETHODCALLTYPE AddRef() noexcept override { return ++refs_; }
  ULONG STDMETHODCALLTYPE Release() noexcept override { auto n = --refs_; if (!n) delete this; return n; }
  HRESULT STDMETHODCALLTYPE SetTotal(const UInt64 *, const UInt64 *) noexcept override {
    return map_.operation->check();
  }
  HRESULT STDMETHODCALLTYPE SetCompleted(const UInt64 *, const UInt64 *) noexcept override {
    return map_.operation->check();
  }
  HRESULT STDMETHODCALLTYPE GetProperty(PROPID id, PROPVARIANT *v) noexcept override {
    if (!v) return map_.operation->finish(E_POINTER);
    PropVariantInit(v);
    return map_.operation->call([&]() -> HRESULT {
      exact(map_.operation->check(), "source operation failed or cancelled");
      if (id == kpidName) {
        v->bstrVal = SysAllocStringLen(map_.names[0].data(), UINT(map_.names[0].size()));
        if (!v->bstrVal) return E_OUTOFMEMORY; v->vt = VT_BSTR;
      } else if (id == kpidSize) {
        v->vt = VT_UI8; v->uhVal.QuadPart = map_.sizes[0];
      } else if (id == kpidIsDir) { v->vt = VT_BOOL; v->boolVal = VARIANT_FALSE; }
      return S_OK;
    });
  }
  HRESULT STDMETHODCALLTYPE GetStream(const wchar_t *name, IInStream **out) noexcept override {
    if (!out) return map_.operation->finish(E_POINTER); *out = nullptr;
    bool predetermined_eof = false;
    HRESULT hr = boundary([&]() -> HRESULT {
      exact(map_.operation->check(), "source operation failed or cancelled");
      if (!volume_requests_ || !name) return E_FAIL;
      size_t n = wcsnlen_s(name, 32768);
      if (!n || n == 32768) return E_FAIL;
      std::wstring leaf(name, n);
      if (leaf == map_.terminal) {
        exact(map_.operation->observe(12, UINT32_MAX, 1, S_FALSE), "trace failure");
        predetermined_eof = true; return S_FALSE;
      }
      auto i = std::find(map_.names.begin(), map_.names.end(), leaf);
      if (i == map_.names.end()) return E_FAIL;
      uint32_t index = uint32_t(i - map_.names.begin());
      exact(map_.operation->observe(12, index), "trace failure");
      *out = new VolumeStream(map_.operation, index, map_.sizes[index]);
      return S_OK;
    });
    if (hr == S_FALSE && predetermined_eof) return S_FALSE; // Frozen EOF, never lookup.
    return map_.operation->finish(hr);
  }
};

class OutputStream final : public ISequentialOutStream {
  std::atomic<ULONG> refs_{1};
  std::shared_ptr<Shared> state_; uint32_t index_; uint64_t token_; bool disk_;
public:
  OutputStream(std::shared_ptr<Shared> s, uint32_t i, uint64_t token, bool disk)
      : state_(std::move(s)), index_(i), token_(token), disk_(disk) { ++state_->streams_open; }
  ~OutputStream() {
    if (disk_) state_->record(state_->host.close_private_output(state_->host.context, token_));
    --state_->streams_open;
  }
  HRESULT STDMETHODCALLTYPE QueryInterface(REFIID iid, void **p) noexcept override {
    if (!p) return E_POINTER; *p = nullptr;
    if (iid != IID_IUnknown && iid != IID_ISequentialOutStream) return E_NOINTERFACE;
    *p = static_cast<ISequentialOutStream *>(this); AddRef(); return S_OK;
  }
  ULONG STDMETHODCALLTYPE AddRef() noexcept override { return ++refs_; }
  ULONG STDMETHODCALLTYPE Release() noexcept override { auto n = --refs_; if (!n) delete this; return n; }
  HRESULT STDMETHODCALLTYPE Write(const void *data, UInt32 size, UInt32 *processed) noexcept override {
    if (processed) *processed = 0;
    HRESULT hr = boundary([&]() -> HRESULT {
      exact(state_->check(), "cancelled");
      exact(state_->operation->observe(24, index_, disk_ ? 1 : 0, 0, size), "trace failure");
      if (size && !data) return E_POINTER;
      {
        std::lock_guard<std::mutex> lock(state_->ledger);
        if (size > state_->items[index_].value.size - state_->bytes[index_]) return E_FAIL;
      }
      uint32_t wrote = size;
      HRESULT result = S_OK;
      if (disk_) {
        wrote = 0;
        result = state_->host.write_private_output(state_->host.context, token_, data, size, &wrote);
      }
      if (wrote > size || (size && !wrote && result == S_OK)) return E_FAIL;
      state_->count(index_, wrote);
      if (processed) *processed = wrote;
      return result;
    });
    state_->record(hr); return hr;
  }
};

class ExtractCallback final : public IArchiveExtractCallback, public IArchiveExtractCallbackMessage2 {
  std::atomic<ULONG> refs_{1};
  std::shared_ptr<Shared> state_;
public:
  explicit ExtractCallback(std::shared_ptr<Shared> s) : state_(std::move(s)) {}
  HRESULT STDMETHODCALLTYPE QueryInterface(REFIID iid, void **p) noexcept override {
    if (!p) return E_POINTER; *p = nullptr;
    if (iid == IID_IUnknown || iid == IID_IProgress || iid == IID_IArchiveExtractCallback)
      *p = static_cast<IArchiveExtractCallback *>(this);
    else if (iid == IID_IArchiveExtractCallbackMessage2)
      *p = static_cast<IArchiveExtractCallbackMessage2 *>(this);
    else return E_NOINTERFACE;
    AddRef(); return S_OK;
  }
  ULONG STDMETHODCALLTYPE AddRef() noexcept override { return ++refs_; }
  ULONG STDMETHODCALLTYPE Release() noexcept override { auto n = --refs_; if (!n) delete this; return n; }
  HRESULT STDMETHODCALLTYPE SetTotal(UInt64 total) noexcept override {
    if (total != state_->total.load()) { state_->record(E_FAIL); return E_FAIL; }
    return state_->check();
  }
  HRESULT STDMETHODCALLTYPE SetCompleted(const UInt64 *) noexcept override {
    state_->host.progress(state_->host.context, state_->decoded.load(), state_->total.load());
    return state_->check();
  }
  HRESULT STDMETHODCALLTYPE GetStream(UInt32 index, ISequentialOutStream **out, Int32 mode) noexcept override {
    if (!out) { state_->record(E_POINTER); return E_POINTER; } *out = nullptr;
    HRESULT hr = boundary([&]() -> HRESULT {
      exact(state_->check(), "cancelled");
      exact(state_->operation->observe(20, index, mode), "trace failure");
      {
        std::lock_guard<std::mutex> lock(state_->ledger);
        if (index >= state_->items.size() || mode != NArchive::NExtract::NAskMode::kExtract ||
            state_->current != NONE || state_->gotten[index]) return E_FAIL;
        state_->gotten[index] = 1; state_->current = index;
      }
      if (state_->items[index].value.is_dir) return S_OK;
      bool disk = state_->selected[index] != 0;
      uint64_t token = 0;
      // Allocate before opening a host file, so allocation failure cannot leak a token.
      auto stream = std::unique_ptr<OutputStream>(new OutputStream(state_, index, token, false));
      if (disk) {
        exact(state_->host.open_private_output(state_->host.context, index, &token), "private output open");
        // Replacing the discard object closes no file. If construction fails, close token.
        try { stream.reset(new OutputStream(state_, index, token, true)); }
        catch (...) { state_->record(state_->host.close_private_output(state_->host.context, token)); throw; }
      }
      *out = stream.release(); // Non-null for EVERY ordinary file, including empty.
      return S_OK;
    });
    state_->record(hr); return hr;
  }
  HRESULT STDMETHODCALLTYPE PrepareOperation(Int32 mode) noexcept override {
    HRESULT hr = boundary([&]() -> HRESULT {
      exact(state_->check(), "cancelled");
      std::lock_guard<std::mutex> lock(state_->ledger);
      uint32_t i = state_->current;
      exact(state_->operation->observe(21, i, mode), "trace failure");
      if (mode != NArchive::NExtract::NAskMode::kExtract || i == NONE || state_->prepared[i]) return E_FAIL;
      state_->prepared[i] = 1; return S_OK;
    });
    state_->record(hr); return hr;
  }
  HRESULT STDMETHODCALLTYPE SetOperationResult(Int32 result) noexcept override {
    HRESULT hr = boundary([&]() -> HRESULT {
      std::lock_guard<std::mutex> lock(state_->ledger);
      uint32_t i = state_->current;
      exact(state_->operation->observe(22, i, 0, result), "trace failure");
      if (i == NONE || !state_->prepared[i] || state_->done[i]) return E_FAIL;
      if (result != NArchive::NExtract::NOperationResult::kOK) {
        ++state_->item_errors; return E_FAIL;
      }
      if (state_->bytes[i] != state_->items[i].value.size) return E_FAIL;
      state_->done[i] = 1; state_->current = NONE;
      return state_->check();
    });
    state_->record(hr); return hr;
  }
  HRESULT STDMETHODCALLTYPE ReportExtractResult(UInt32 type, UInt32 index, Int32 result) noexcept override {
    HRESULT trace = state_->operation->observe(23, index, int32_t(type), result);
    if (trace != S_OK) return trace;
    if (type > NArchive::NEventIndexType::kOutArcIndex ||
        ((type == NArchive::NEventIndexType::kInArcIndex || type == NArchive::NEventIndexType::kOutArcIndex)
         && index >= state_->items.size()) || result != NArchive::NExtract::NOperationResult::kOK) {
      ++state_->block_errors; state_->record(E_FAIL); return E_FAIL;
    }
    return state_->check();
  }
};

struct Runtime {
  using K7Init = int32_t (WINAPI *)();
  using K7Allowed = int32_t (WINAPI *)(const char *);
  using NumMethods = HRESULT (WINAPI *)(UInt32 *);
  using MethodProperty = HRESULT (WINAPI *)(UInt32, PROPID, PROPVARIANT *);
  std::vector<HMODULE> retained; // One task per worker; process exit owns unload.
  Func_CreateObject create = nullptr;
  Func_GetNumberOfFormats count = nullptr;
  Func_GetHandlerProperty2 property = nullptr;
  K7Allowed allowed_handler = nullptr;
  template<class F> F symbol(HMODULE m, const char *n) {
    auto p = GetProcAddress(m, n); if (!p) fail("missing official export");
    return reinterpret_cast<F>(p);
  }
  HMODULE load(const std::wstring &dir, const wchar_t *leaf) {
    HMODULE m = LoadLibraryExW((dir + L"\\" + leaf).c_str(), nullptr, LOAD_LIBRARY_SEARCH_SYSTEM32);
    if (!m) fail("official runtime load failed", HRESULT_FROM_WIN32(GetLastError()));
    retained.push_back(m); return m;
  }
  void init(const wchar_t *directory) {
    if (!directory || wcsnlen_s(directory, 32768) == 32768) fail("invalid runtime directory");
    std::wstring dir(directory);
    if (dir.size() < 3 || dir[1] != L':' || dir[2] != L'\\') fail("runtime directory must be trusted absolute local path");
    for (const wchar_t *leaf : {L"K7Base.dll", L"NanaZip.Codecs.dll", L"NanaZip.Core.dll", L"NanaZip.ZS.Codecs.dll"})
      if (GetModuleHandleW(leaf)) fail("archive runtime was loaded before worker initialization");
    HMODULE k7 = load(dir, L"K7Base.dll");
    exact(symbol<K7Init>(k7, "K7BaseInitialize")(), "K7 initialization failed");
    allowed_handler = symbol<K7Allowed>(k7, "K7BaseGetAllowedHandlerPolicy");
    load(dir, L"NanaZip.Codecs.dll");
    HMODULE core = load(dir, L"NanaZip.Core.dll");
    load(dir, L"NanaZip.ZS.Codecs.dll");
    create = symbol<Func_CreateObject>(core, "CreateObject");
    count = symbol<Func_GetNumberOfFormats>(core, "GetNumberOfFormats");
    property = symbol<Func_GetHandlerProperty2>(core, "GetHandlerProperty2");
    // First subset uses Core-local built-ins only. No claim of external SetCodecs support.
    UInt32 n = 0; exact(symbol<NumMethods>(core, "GetNumberOfMethods")(&n), "codec enumeration");
    if (n > 4096) fail("codec enumeration limit");
    auto prop = symbol<MethodProperty>(core, "GetMethodProperty");
    bool copy = false, lzma = false, lzma2 = false;
    for (UInt32 i = 0; i < n; ++i) {
      Variant v; exact(prop(i, NMethodPropID::kName, &v.v), "codec name");
      auto name = text(v.v);
      copy |= name == L"Copy"; lzma |= name == L"LZMA"; lzma2 |= name == L"LZMA2";
    }
    if (!copy || !lzma || !lzma2) fail("required Core built-in codec missing");
  }
  void handler(const wchar_t *name, const char *policy_name, IInArchive **out) {
    if (!allowed_handler(policy_name)) fail("handler denied by official policy");
    UInt32 n = 0; exact(count(&n), "handler enumeration");
    if (n > 256) fail("handler enumeration limit");
    for (UInt32 i = 0; i < n; ++i) {
      Variant v; exact(property(i, NArchive::NHandlerPropID::kName, &v.v), "handler name");
      if (text(v.v) != name) continue;
      Variant cls; exact(property(i, NArchive::NHandlerPropID::kClassID, &cls.v), "handler class");
      if (cls.v.vt != VT_BSTR || SysStringByteLen(cls.v.bstrVal) != sizeof(GUID)) fail("invalid class GUID");
      GUID guid; std::memcpy(&guid, cls.v.bstrVal, sizeof(guid));
      exact(create(&guid, &IID_IInArchive, reinterpret_cast<void **>(out)), "handler creation");
      if (!*out) fail("null handler"); return;
    }
    fail("required official handler unavailable");
  }
};

// Restrict the direct handler path to systems using the standard format inventory.
// Only definitely empty/missing format lists qualify. Active/ambiguous policy is
// fatal, never a generic fallback to a compatibility engine.
bool effective_policy_or_unknown() {
  HKEY key = nullptr;
  LSTATUS rc = RegOpenKeyExW(HKEY_LOCAL_MACHINE, L"Software\\Policies\\M2Team\\NanaZip", 0,
                           KEY_READ | KEY_WOW64_64KEY, &key);
  if (rc == ERROR_FILE_NOT_FOUND || rc == ERROR_PATH_NOT_FOUND) return false;
  if (rc != ERROR_SUCCESS) return true;
  struct Close { HKEY h; ~Close() { RegCloseKey(h); } } close{key};
  for (const wchar_t *name : {L"AllowedHandlers", L"BlockedHandlers", L"AllowedCodecs", L"BlockedCodecs"}) {
    DWORD type = 0, bytes = 0;
    rc = RegQueryValueExW(key, name, nullptr, &type, nullptr, &bytes);
    if (rc == ERROR_FILE_NOT_FOUND) continue;
    if (rc != ERROR_SUCCESS || type != REG_MULTI_SZ || bytes > 65536 || bytes % 2) return true;
    if (!bytes) continue;
    std::vector<wchar_t> data(bytes / 2, 0);
    DWORD received = bytes;
    rc = RegQueryValueExW(key, name, nullptr, &type, reinterpret_cast<BYTE *>(data.data()), &received);
    if (rc != ERROR_SUCCESS || type != REG_MULTI_SZ || received != bytes) return true;
    if (data[0] != L'\0') return true;
  }
  return false;
}

void archive_health(IInArchive *arc) {
  for (PROPID id : {kpidErrorFlags, kpidWarningFlags, kpidNumErrors, kpidTailSize}) {
    Variant v; exact(arc->GetArchiveProperty(id, &v.v), "archive status query");
    if (number(v.v, true)) fail("archive reports errors warnings or tail");
  }
  for (PROPID id : {kpidError, kpidWarning}) {
    Variant v; exact(arc->GetArchiveProperty(id, &v.v), "archive diagnostic query");
    if (!text(v.v, true).empty()) fail("archive reports diagnostic text");
  }
}
// The official 7z handler renders a single LZMA2 coder as LZMA2:19,
// LZMA2:768k, etc. A space separates coders in a filter/encryption chain.
// The decoder itself owns dictionary properties and stream validation.
bool single_lzma2(std::wstring_view method) {
  constexpr std::wstring_view prefix = L"LZMA2:";
  if (!method.starts_with(prefix)) return false;
  method.remove_prefix(prefix.size());
  if (method.empty() || method.size() > 5) return false;
  if (method.back() == L'k' || method.back() == L'm') method.remove_suffix(1);
  return !method.empty() && std::all_of(method.begin(), method.end(),
      [](wchar_t c) { return c >= L'0' && c <= L'9'; });
}
void manifest(IInArchive *arc, Shared &state, uint64_t source_size) {
  archive_health(arc);
  { Variant v; exact(arc->GetArchiveProperty(kpidOffset, &v.v), "archive offset");
    if (number(v.v, true)) fail("embedded archive is outside supported method set"); }
  { Variant v; exact(arc->GetArchiveProperty(kpidPhySize, &v.v), "archive physical size");
    if (number(v.v) != source_size) fail("fixed volumes do not exactly contain archive"); }
  bool solid = false, has_copy = false, has_lzma2 = false;
  { Variant v; exact(arc->GetArchiveProperty(kpidSolid, &v.v), "archive solid flag");
    solid = boolean(v.v, false); }
  UInt32 n = 0; exact(arc->GetNumberOfItems(&n), "manifest item count");
  if (!n || n > MAX_ITEMS) fail("manifest size limit");
  state.items.reserve(n); uint64_t name_units = 0, total = 0;
  for (UInt32 i = 0; i < n; ++i) {
    exact(state.check(), "cancelled");
    Item item; item.value.index = i;
    { Variant v; exact(arc->GetProperty(i, kpidPath, &v.v), "item path"); item.name = text(v.v); }
    if (item.name.empty() || item.name.size() > MAX_MANIFEST_UNITS - name_units) fail("manifest names limit");
    name_units += item.name.size();
    { Variant v; exact(arc->GetProperty(i, kpidIsDir, &v.v), "item type"); item.value.is_dir = boolean(v.v); }
    { Variant v; exact(arc->GetProperty(i, kpidSize, &v.v), "item size");
      item.value.size = number(v.v, item.value.is_dir != 0);
      if (item.value.is_dir && item.value.size) fail("directory has payload"); }
    for (PROPID id : {kpidEncrypted, kpidIsAnti, kpidIsAltStream, kpidIsAux, kpidIsDeleted}) {
      Variant v; exact(arc->GetProperty(i, id, &v.v), "item safety property");
      if (boolean(v.v)) fail("item is outside safe supported method set");
    }
    for (PROPID id : {kpidSymLink, kpidHardLink, kpidNtReparse}) {
      Variant v; exact(arc->GetProperty(i, id, &v.v), "item link or security property");
      if (v.v.vt != VT_EMPTY) fail("link or security metadata outside subset");
    }
    { Variant v; exact(arc->GetProperty(i, kpidNtSecure, &v.v), "item security property");
      if (v.v.vt != VT_EMPTY) throw UnsupportedLayout{"archived security descriptor"}; }
    { Variant v; exact(arc->GetProperty(i, kpidMethod, &v.v), "item method");
      auto method = text(v.v, item.value.is_dir || item.value.size == 0);
      if (method == L"Copy") has_copy = true;
      else if (single_lzma2(method)) has_lzma2 = true;
      else if (item.value.size || !method.empty()) fail("unexpected 7z item method"); }
    { Variant v; exact(arc->GetProperty(i, kpidAttrib, &v.v), "item attributes");
      if (v.v.vt != VT_EMPTY) {
        if (v.v.vt != VT_UI4) fail("invalid attributes type");
        item.value.attrs_present = 1; item.value.attrs = v.v.ulVal;
        if (item.value.attrs & FILE_ATTRIBUTE_REPARSE_POINT) fail("reparse attribute");
      } }
    { Variant v; exact(arc->GetProperty(i, kpidCRC, &v.v), "item CRC property");
      if (v.v.vt != VT_EMPTY) {
        if (v.v.vt != VT_UI4) fail("invalid CRC property type");
        item.value.crc_present = 1; item.value.crc = v.v.ulVal;
      } }
    { Variant v; exact(arc->GetProperty(i, kpidCTime, &v.v), "item creation time"); item.value.ctime = time_value(v.v); }
    { Variant v; exact(arc->GetProperty(i, kpidATime, &v.v), "item access time"); item.value.atime = time_value(v.v); }
    { Variant v; exact(arc->GetProperty(i, kpidMTime, &v.v), "item modification time"); item.value.mtime = time_value(v.v); }
    if (item.value.size > UINT64_MAX - total) fail("total size overflow"); total += item.value.size;
    state.items.push_back(std::move(item));
  }
  // Only positive layout identifications may defer to the existing decoder.
  // Decide before the manifest ACK, granting output handles, or extracting.
  if ((solid && !has_lzma2) || (has_copy && has_lzma2))
    throw UnsupportedLayout{"unsupported 7z method combination"};
  state.total = total;
  state.gotten.resize(n); state.prepared.resize(n); state.done.resize(n); state.bytes.resize(n);
  std::vector<QzSelectedItem> values; values.reserve(n);
  for (auto &item : state.items) {
    item.value.name = item.name.data(); item.value.name_units = uint32_t(item.name.size());
    values.push_back(item.value);
  }
  std::vector<uint8_t> acknowledgement(n, 0xFF);
  exact(state.host.accept_manifest(state.host.context, values.data(), n, acknowledgement.data()), "manifest rejected by trusted host");
  bool any = false;
  for (uint8_t b : acknowledgement) { if (b > 1) fail("incomplete or invalid selection acknowledgement"); any |= b != 0; }
  if (!any) fail("no requested archive entries matched");
  state.selected.assign(acknowledgement.begin(), acknowledgement.end()); // Host never saw this storage.
}
void reason(QzSelectedReceipt &r, const char *s) { strncpy_s(r.reason, sizeof(r.reason), s, _TRUNCATE); }
} // namespace

extern "C" HRESULT __stdcall qz_extract_selected(const QzSelectedRequest *request, QzSelectedReceipt *receipt) {
  if (!receipt) return E_POINTER;
  *receipt = {}; receipt->outcome = QZ_SELECTED_FAILED; receipt->result = E_FAIL;
  std::shared_ptr<OperationStatus> operation;
  std::shared_ptr<Shared> state;
  try {
    static std::atomic<bool> called{false};
    if (called.exchange(true)) fail("one task per isolated worker required");
    if (!request || request->abi_version != 1 || request->struct_bytes != sizeof(QzSelectedRequest) || !request->host)
      fail("invalid request ABI");
    const auto host_snapshot = *request->host;
    const auto &host = host_snapshot;
    if (host.abi_version != 1 || host.struct_bytes != sizeof(QzSelectedHost) || !host.read_volume_at ||
        !host.accept_manifest || !host.prepare_private_tree || !host.open_private_output || !host.write_private_output ||
        !host.close_private_output || !host.cancelled || !host.progress || !host.trace) fail("invalid host ABI");
    if (host.cancelled(host.context)) fail("cancelled", E_ABORT);
    operation = std::make_shared<OperationStatus>(host);
    VolumeMap map(*request, operation); // Own names/sizes; no request-array reads later.
    if (effective_policy_or_unknown()) {
      if (host.cancelled(host.context)) fail("cancelled", E_ABORT);
      fail("active or unknown policy: selected worker unavailable", E_ACCESSDENIED);
    }
    Runtime runtime; runtime.init(request->sealed_runtime_directory);
    // Recheck conservatively before Open; no claim of atomic policy epochs.
    if (effective_policy_or_unknown()) {
      if (host.cancelled(host.context)) fail("cancelled", E_ABORT);
      fail("policy changed before Open: selected worker unavailable", E_ACCESSDENIED);
    }
    Ref<IInArchive> split; runtime.handler(L"Split", "Split", split.put());
    Ref<IInStream> first(new VolumeStream(operation, 0, map.sizes[0]));
    Ref<IArchiveOpenCallback> outer_callback(new OpenCallback(map, true));
    UInt64 start = 0;
    exact(split->Open(first.get(), &start, outer_callback.get()), "Split Open failed");
    archive_health(split.get());
    UInt32 split_items = 0; exact(split->GetNumberOfItems(&split_items), "Split item count");
    if (split_items != 1) fail("unexpected Split root");
    { Variant v; exact(split->GetArchiveProperty(kpidMainSubfile, &v.v), "Split main item");
      if (number(v.v) != 0) fail("unexpected Split main item"); }
    Ref<IInArchiveGetStream> joined_provider;
    exact(split->QueryInterface(IID_IInArchiveGetStream, reinterpret_cast<void **>(joined_provider.put())), "Split stream interface");
    Ref<ISequentialInStream> joined_sequential;
    exact(joined_provider->GetStream(0, joined_sequential.put()), "Split joined stream");
    if (!joined_sequential.get()) fail("null Split joined stream");
    Ref<IInStream> joined;
    exact(joined_sequential->QueryInterface(IID_IInStream, reinterpret_cast<void **>(joined.put())), "Split stream must be seekable");
    Ref<IInArchive> inner; runtime.handler(L"7z", "7z", inner.put());
    Ref<IArchiveOpenCallback> inner_callback(new OpenCallback(map, false));
    exact(inner->Open(joined.get(), &start, inner_callback.get()), "7z Open failed");
    state = std::make_shared<Shared>(operation);
    manifest(inner.get(), *state, map.total);
    exact(state->check(), "cancelled");
    exact(host.prepare_private_tree(host.context), "private selected directory preparation");
    Ref<IArchiveExtractCallback> callback(new ExtractCallback(state));
    exact(inner->Extract(nullptr, UINT32_MAX, 0, callback.get()), "official all-item extraction failed");
    exact(state->check(), "source callback failed or cancelled");
    archive_health(inner.get());
    if (state->current != NONE || state->streams_open.load() || state->item_errors.load() || state->block_errors.load() ||
        state->decoded.load() != state->total.load() || std::find(state->done.begin(), state->done.end(), 0) != state->done.end())
      fail("incomplete all-item completion ledger");
    exact(inner->Close(), "7z Close failed"); exact(split->Close(), "Split Close failed");
    exact(operation->check(), "callback failed or cancelled during Close");
    if (state->streams_open.load()) fail("output streams retained after Close");
    receipt->outcome = QZ_SELECTED_DECODED_PRIVATE; receipt->result = S_OK;
    reason(*receipt, "private decode only; worker exit and host verification still required");
  } catch (const UnsupportedLayout &e) {
    receipt->outcome = QZ_SELECTED_UNSUPPORTED_LAYOUT; receipt->result = S_OK; reason(*receipt, e.reason);
  } catch (const Failure &e) { receipt->result = FAILED(e.hr) ? e.hr : E_FAIL; reason(*receipt, e.reason); }
  catch (const std::bad_alloc &) { receipt->result = E_OUTOFMEMORY; reason(*receipt, "allocation failed"); }
  catch (...) { receipt->result = E_FAIL; reason(*receipt, "bridge exception"); }
  // All local COM references have now been released. Catch late close/destructor
  // callbacks too; parent still requires complete IPC AND actual worker exit.
  if (operation && operation->check() != S_OK) {
    receipt->outcome = QZ_SELECTED_FAILED;
    receipt->result = operation->first_error.load();
    reason(*receipt, "sticky source callback failure or cancellation");
  }
  if (state && receipt->outcome != QZ_SELECTED_UNSUPPORTED_LAYOUT) {
    receipt->item_count = uint32_t(state->items.size());
    receipt->completion_count = uint32_t(std::count(state->done.begin(), state->done.end(), uint8_t(1)));
    receipt->item_errors = state->item_errors; receipt->block_errors = state->block_errors;
    receipt->streams_open = state->streams_open;
    receipt->decoded_bytes = state->decoded; receipt->selected_bytes = state->selected_bytes;
  }
  return receipt->result;
}
