// Create new, single-volume Deflate archives from pinned local NTFS inputs.
#include <array>
#include <winternl.h>

namespace zip_create {
struct NotEligible {};
void frame(BYTE type, const BYTE *body = nullptr, BYTE size = 0) {
  BYTE buffer[6] = {type, size, 0, 0, 0, 0};
  if (size)
    buffer[5] = *body;
  DWORD n = 0;
  if (!WriteFile(GetStdHandle(STD_OUTPUT_HANDLE), buffer, 5u + size, &n,
                 nullptr) ||
      n != 5u + size)
    fail("ZIP worker control channel closed");
}
void win(BOOL ok, const char *message) {
  if (!ok)
    fail(message, HRESULT_FROM_WIN32(GetLastError()));
}
struct Close {
  void operator()(void *h) const noexcept {
    if (h && h != INVALID_HANDLE_VALUE)
      CloseHandle(h);
  }
};
using File = std::unique_ptr<void, Close>;
uint64_t ticks(FILETIME x) {
  return uint64_t(x.dwHighDateTime) << 32 | x.dwLowDateTime;
}
struct Record {
  std::wstring name, path;
  HANDLE parent;
  uint64_t id, size, modified, created, accessed;
  DWORD attributes;
  bool directory() const {
    return (attributes & FILE_ATTRIBUTE_DIRECTORY) != 0;
  }
};
bool valid_leaf(std::wstring_view name) {
  if (name.empty() || name.size() > 255 || name.back() == L'.' ||
      name.back() == L' ' || name.find_first_of(L"\\/:*?\"<>|") != name.npos ||
      std::any_of(name.begin(), name.end(), [](wchar_t c) { return c < 32; }))
    return false;
  auto stem = name.substr(0, name.find(L'.'));
  while (!stem.empty() && stem.back() == L' ')
    stem.remove_suffix(1);
  auto equal = [](std::wstring_view a, std::wstring_view b) {
    return a.size() == b.size() &&
           CompareStringOrdinal(a.data(), int(a.size()), b.data(),
                                int(b.size()), TRUE) == CSTR_EQUAL;
  };
  for (const auto *device :
       {L"CON", L"PRN", L"NUL", L"AUX", L"CONIN$", L"CONOUT$", L"CLOCK$"})
    if (equal(stem, device))
      return false;
  if (stem.size() == 4 &&
      (equal(stem.substr(0, 3), L"COM") || equal(stem.substr(0, 3), L"LPT"))) {
    wchar_t c = stem.back();
    if ((c >= L'0' && c <= L'9') || c == 0xb9 || c == 0xb2 || c == 0xb3)
      return false;
  }
  return true;
}
class Source {
  using Open = NTSTATUS(NTAPI *)(PHANDLE, ACCESS_MASK, POBJECT_ATTRIBUTES,
                                 PIO_STATUS_BLOCK, PLARGE_INTEGER, ULONG, ULONG,
                                 ULONG, ULONG, PVOID, ULONG);
  Open open_;
  HANDLE owner_;
  std::vector<File> pins_;
  size_t units_ = 0;
  uint64_t total_ = 0;
  void append(Record r) {
    if (r.attributes & FILE_ATTRIBUTE_REPARSE_POINT)
      fail("linked input path rejected");
    if (!valid_leaf(r.name) || r.path.size() > 32767 ||
        items.size() >= 100000 || (units_ += r.path.size()) > 8 * 1024 * 1024 ||
        (r.attributes & ~DWORD(0xb7)))
      throw NotEligible{};
    // ZIP64 and untested sparse/offline storage use the established console.
    if (!r.directory() && (r.size > UINT32_MAX || total_ > UINT32_MAX - r.size))
      throw NotEligible{};
    if (!r.directory())
      total_ += r.size;
    items.emplace_back(std::move(r));
  }

public:
  std::vector<Record> items;
  explicit Source(HANDLE owner) : owner_(owner) {
    open_ = reinterpret_cast<Open>(
        GetProcAddress(GetModuleHandleW(L"ntdll.dll"), "NtCreateFile"));
    if (!open_)
      fail("NtCreateFile unavailable");
  }
  void live() const {
    if (WaitForSingleObject(owner_, 0) != WAIT_TIMEOUT)
      fail("parent exited", E_ABORT);
  }
  File current(const Record &r) const {
    live();
    UNICODE_STRING name{};
    name.Buffer = const_cast<wchar_t *>(r.name.data());
    name.Length = name.MaximumLength =
        static_cast<USHORT>(r.name.size() * sizeof(wchar_t));
    OBJECT_ATTRIBUTES object{};
    object.Length = sizeof(object);
    object.RootDirectory = r.parent;
    object.ObjectName = &name;
    object.Attributes =
        0x1000; // OBJ_DONT_REPARSE; use actual enumerated casing.
    IO_STATUS_BLOCK status{};
    HANDLE h = nullptr;
    ULONG flags =
        0x00200000 | 0x20 |
        (r.directory()
             ? 1
             : 0x40); // OPEN_REPARSE_POINT, synchronous, required type.
    NTSTATUS opened = open_(&h, 0x100081, &object, &status, nullptr, 0,
                            FILE_SHARE_READ, 1, flags, nullptr, 0);
    if (opened < 0) {
      // Directory handles are acquired during enumeration only. Shared writers
      // can make strong pins unavailable even when console validation succeeds.
      if (r.directory() &&
          (ULONG(opened) == 0xc0000043 || ULONG(opened) == 0xc0000022))
        throw NotEligible{};
      fail("cannot safely open enumerated source");
    }
    File file(h);
    BY_HANDLE_FILE_INFORMATION i{};
    win(GetFileInformationByHandle(h, &i), "source file information");
    uint64_t id = (uint64_t(i.nFileIndexHigh) << 32) | i.nFileIndexLow;
    uint64_t size = (uint64_t(i.nFileSizeHigh) << 32) | i.nFileSizeLow;
    if (GetFileType(h) != FILE_TYPE_DISK ||
        (i.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT))
      fail("source is not an ordinary disk entry");
    if (((i.dwFileAttributes ^ r.attributes) & FILE_ATTRIBUTE_DIRECTORY) ||
        id != r.id)
      fail("source identity changed");
    if (!r.directory() &&
        (i.dwFileAttributes != r.attributes ||
         ticks(i.ftLastWriteTime) != r.modified || size != r.size))
      fail("source size or timestamp changed");
    return file;
  }
  void enumerate(HANDLE root, std::wstring name) {
    wchar_t fs[32]{};
    win(GetVolumeInformationByHandleW(root, nullptr, 0, nullptr, nullptr,
                                      nullptr, fs, 32),
        "source filesystem");
    if (_wcsicmp(fs, L"NTFS") != 0)
      throw NotEligible{};
    BY_HANDLE_FILE_INFORMATION i{};
    win(GetFileInformationByHandle(root, &i), "root metadata");
    if ((i.dwFileAttributes &
         (FILE_ATTRIBUTE_REPARSE_POINT | FILE_ATTRIBUTE_DIRECTORY)) !=
        FILE_ATTRIBUTE_DIRECTORY)
      fail("invalid source root");
    append({name, name, nullptr,
            (uint64_t(i.nFileIndexHigh) << 32) | i.nFileIndexLow, 0,
            ticks(i.ftLastWriteTime), ticks(i.ftCreationTime),
            ticks(i.ftLastAccessTime), i.dwFileAttributes});
    struct Folder {
      HANDLE h;
      std::wstring path;
    };
    std::vector<Folder> work{{root, name}};
    alignas(8) std::array<BYTE, 64 * 1024> buffer;
    while (!work.empty()) {
      Folder dir = std::move(work.back());
      work.pop_back();
      FILE_CASE_SENSITIVE_INFO cs{};
      if (!GetFileInformationByHandleEx(dir.h, FileCaseSensitiveInfo, &cs,
                                        sizeof(cs)) ||
          cs.Flags)
        throw NotEligible{};
      bool first = true;
      for (;;) {
        live();
        if (!GetFileInformationByHandleEx(dir.h,
                                          first ? FileIdBothDirectoryRestartInfo
                                                : FileIdBothDirectoryInfo,
                                          buffer.data(),
                                          static_cast<DWORD>(buffer.size()))) {
          if (GetLastError() == ERROR_NO_MORE_FILES)
            break;
          fail("source directory enumeration failed",
               HRESULT_FROM_WIN32(GetLastError()));
        }
        first = false;
        size_t offset = 0;
        for (;;) {
          constexpr auto header = offsetof(FILE_ID_BOTH_DIR_INFO, FileName);
          if (offset + header > buffer.size())
            fail("invalid enumeration record");
          auto &e = *reinterpret_cast<const FILE_ID_BOTH_DIR_INFO *>(
              buffer.data() + offset);
          if (e.FileNameLength % 2 ||
              e.FileNameLength > buffer.size() - offset - header)
            fail("invalid source name length");
          std::wstring child(e.FileName, e.FileNameLength / 2);
          if (child != L"." && child != L"..") {
            Record r{child,
                     dir.path + L"\\" + child,
                     dir.h,
                     uint64_t(e.FileId.QuadPart),
                     uint64_t(e.EndOfFile.QuadPart),
                     uint64_t(e.LastWriteTime.QuadPart),
                     uint64_t(e.CreationTime.QuadPart),
                     uint64_t(e.LastAccessTime.QuadPart),
                     e.FileAttributes};
            append(std::move(r));
            auto &entry = items.back();
            if (entry.directory()) {
              if (pins_.size() >= 4095)
                throw NotEligible{};
              auto pin = current(entry);
              BY_HANDLE_FILE_INFORMATION actual{};
              win(GetFileInformationByHandle(pin.get(), &actual),
                  "pinned directory metadata");
              entry.modified = ticks(actual.ftLastWriteTime);
              entry.created = ticks(actual.ftCreationTime);
              entry.accessed = ticks(actual.ftLastAccessTime);
              entry.attributes = actual.dwFileAttributes;
              work.push_back({pin.get(), entry.path});
              pins_.emplace_back(std::move(pin));
            }
          }
          if (!e.NextEntryOffset)
            break;
          if (e.NextEntryOffset < header || e.NextEntryOffset % 8)
            fail("invalid directory record offset");
          offset += e.NextEntryOffset;
        }
      }
    }
    std::sort(items.begin(), items.end(), [](const Record &a, const Record &b) {
      return CompareStringOrdinal(a.path.c_str(), -1, b.path.c_str(), -1,
                                  TRUE) == CSTR_LESS_THAN;
    });
    for (size_t j = 1; j < items.size(); ++j) {
      if (CompareStringOrdinal(items[j - 1].path.c_str(), -1,
                               items[j].path.c_str(), -1, TRUE) == CSTR_EQUAL)
        throw NotEligible{};
    }
  }
};

template <class I> class ComObject : public I {
  std::atomic<ULONG> refs_{1};

public:
  virtual ~ComObject() = default;
  ULONG STDMETHODCALLTYPE AddRef() noexcept override { return ++refs_; }
  ULONG STDMETHODCALLTYPE Release() noexcept override {
    auto n = --refs_;
    if (!n)
      delete this;
    return n;
  }

protected:
  HRESULT identify(void **out, bool supported) {
    if (!out)
      return E_POINTER;
    *out = nullptr;
    if (!supported)
      return E_NOINTERFACE;
    *out = static_cast<I *>(this);
    AddRef();
    return S_OK;
  }
};
class Input final : public ComObject<IInStream> {
  File f_;
  uint64_t size_, position_ = 0;
  const Source &source_;

public:
  Input(File f, uint64_t size, const Source &s)
      : f_(std::move(f)), size_(size), source_(s) {}
  HRESULT STDMETHODCALLTYPE QueryInterface(REFIID i,
                                           void **out) noexcept override {
    return identify(out, i == IID_IUnknown || i == IID_ISequentialInStream ||
                             i == IID_IInStream);
  }
  HRESULT STDMETHODCALLTYPE Read(void *buf, UInt32 n,
                                 UInt32 *read) noexcept override {
    if (read)
      *read = 0;
    return boundary([&]() -> HRESULT {
      source_.live();
      DWORD got = 0, requested = static_cast<DWORD>(std::min<uint64_t>(
                         position_ >= size_ ? 0 : size_ - position_, n));
      if (requested) {
        win(ReadFile(f_.get(), buf, requested, &got, nullptr), "source read");
        if (!got)
          fail("source unexpectedly shortened");
      }
      position_ += got;
      if (read)
        *read = got;
      return S_OK;
    });
  }
  HRESULT STDMETHODCALLTYPE Seek(Int64 offset, UInt32 origin,
                                 UInt64 *pos) noexcept override {
    LARGE_INTEGER d{}, result{};
    d.QuadPart = offset;
    if (!SetFilePointerEx(f_.get(), d, &result, origin))
      return HRESULT_FROM_WIN32(GetLastError());
    position_ = uint64_t(result.QuadPart);
    if (pos)
      *pos = position_;
    return S_OK;
  }
};
class Output final : public ComObject<IOutStream> {
  HANDLE h_;

public:
  explicit Output(HANDLE h) : h_(h) {}
  HRESULT STDMETHODCALLTYPE QueryInterface(REFIID i,
                                           void **out) noexcept override {
    return identify(out, i == IID_IUnknown || i == IID_ISequentialOutStream ||
                             i == IID_IOutStream);
  }
  HRESULT STDMETHODCALLTYPE Write(const void *p, UInt32 n,
                                  UInt32 *written) noexcept override {
    DWORD got = 0;
    if (written)
      *written = 0;
    if (!WriteFile(h_, p, n, &got, nullptr))
      return HRESULT_FROM_WIN32(GetLastError());
    if (written)
      *written = got;
    return got == n ? S_OK : E_FAIL;
  }
  HRESULT STDMETHODCALLTYPE Seek(Int64 offset, UInt32 origin,
                                 UInt64 *pos) noexcept override {
    LARGE_INTEGER d{}, result{};
    d.QuadPart = offset;
    if (!SetFilePointerEx(h_, d, &result, origin))
      return HRESULT_FROM_WIN32(GetLastError());
    if (pos)
      *pos = result.QuadPart;
    return S_OK;
  }
  HRESULT STDMETHODCALLTYPE SetSize(UInt64 n) noexcept override {
    FILE_END_OF_FILE_INFO end{};
    end.EndOfFile.QuadPart = n;
    return SetFileInformationByHandle(h_, FileEndOfFileInfo, &end, sizeof(end))
               ? S_OK
               : HRESULT_FROM_WIN32(GetLastError());
  }
};
class Update final : public ComObject<IArchiveUpdateCallback> {
  Source &src_;
  std::mutex progress_;
  UInt64 total_ = 0;
  BYTE last_ = 0;

public:
  std::atomic<HRESULT> error{S_OK};
  explicit Update(Source &s) : src_(s) {}
  HRESULT STDMETHODCALLTYPE QueryInterface(REFIID i,
                                           void **out) noexcept override {
    return identify(out, i == IID_IUnknown || i == IID_IProgress ||
                             i == IID_IArchiveUpdateCallback);
  }
  HRESULT STDMETHODCALLTYPE SetTotal(UInt64 value) noexcept override {
    return boundary([&]() {
      src_.live();
      std::lock_guard<std::mutex> lock(progress_);
      total_ = value;
      return S_OK;
    });
  }
  HRESULT STDMETHODCALLTYPE
  SetCompleted(const UInt64 *value) noexcept override {
    return boundary([&]() {
      src_.live();
      std::lock_guard<std::mutex> lock(progress_);
      if (value && total_) {
        BYTE n = static_cast<BYTE>(
            std::min(99.0, 100.0 * double(*value) / double(total_)));
        if (n > last_) {
          frame(46, &n, 1);
          last_ = n;
        }
      }
      return S_OK;
    });
  }
  HRESULT STDMETHODCALLTYPE GetUpdateItemInfo(UInt32 i, Int32 *data,
                                              Int32 *props,
                                              UInt32 *old) noexcept override {
    if (i >= src_.items.size())
      return E_INVALIDARG;
    if (data)
      *data = 1;
    if (props)
      *props = 1;
    if (old)
      *old = UINT32_MAX;
    return S_OK;
  }
  HRESULT STDMETHODCALLTYPE GetProperty(UInt32 i, PROPID id,
                                        PROPVARIANT *v) noexcept override {
    if (!v || i >= src_.items.size())
      return E_INVALIDARG;
    const auto &r = src_.items[i];
    PropVariantInit(v);
    switch (id) {
    case kpidPath:
      v->vt = VT_BSTR;
      v->bstrVal =
          SysAllocStringLen(r.path.data(), static_cast<UINT>(r.path.size()));
      return v->bstrVal ? S_OK : E_OUTOFMEMORY;
    case kpidIsDir:
      v->vt = VT_BOOL;
      v->boolVal = r.directory() ? VARIANT_TRUE : VARIANT_FALSE;
      break;
    case kpidSize:
      v->vt = VT_UI8;
      v->uhVal.QuadPart = r.directory() ? 0 : r.size;
      break;
    case kpidAttrib:
      v->vt = VT_UI4;
      v->ulVal = r.attributes;
      break;
    case kpidMTime:
    case kpidCTime:
    case kpidATime: {
      const auto value = id == kpidMTime
                             ? r.modified
                             : (id == kpidCTime ? r.created : r.accessed);
      v->vt = VT_FILETIME;
      v->filetime.dwLowDateTime = static_cast<DWORD>(value);
      v->filetime.dwHighDateTime = value >> 32;
      break;
    }
    }
    return S_OK;
  }
  HRESULT STDMETHODCALLTYPE
  GetStream(UInt32 i, ISequentialInStream **out) noexcept override {
    if (!out || i >= src_.items.size())
      return E_INVALIDARG;
    *out = nullptr;
    const auto &r = src_.items[i];
    if (r.directory())
      return S_OK;
    auto status = boundary([&]() {
      *out = new Input(src_.current(r), r.size, src_);
      return S_OK;
    });
    if (FAILED(status))
      error.store(status);
    return status;
  }
  HRESULT STDMETHODCALLTYPE SetOperationResult(Int32 result) noexcept override {
    if (result != 0)
      error.store(E_FAIL);
    return error.load();
  }
};
} // namespace zip_create

extern "C" HRESULT qz_create_zip(const wchar_t *runtime, HANDLE owner,
                                 HANDLE root, const wchar_t *name,
                                 HANDLE output, DWORD level, DWORD threads,
                                 DWORD explicit_method, char *message,
                                 size_t capacity) noexcept {
  using namespace zip_create;
  try {
    if (!runtime || !owner || !root || !name || !output ||
        (level != UINT32_MAX && (level < 1 || level > 9)) || threads > 64 ||
        explicit_method > 1)
      fail("invalid ZIP request");
    if (qing_machine_policy_restricts_native())
      fail("machine policy rejects native route");
    Runtime dll;
    dll.init(runtime);
    Ref<IOutArchive> archive;
    dll.handler(L"zip", "Zip", archive.put(), IID_IOutArchive);
    Ref<ISetProperties> settings;
    exact(archive->QueryInterface(IID_ISetProperties,
                                  reinterpret_cast<void **>(settings.put())),
          "ZIP options interface");
    const wchar_t *keys[3]{};
    PROPVARIANT props[3]{};
    unsigned n = 0;
    if (level != UINT32_MAX) {
      keys[n] = L"x";
      props[n].vt = VT_UI4;
      props[n++].ulVal = level;
    }
    Variant method;
    if (explicit_method) {
      keys[n] = L"m";
      method.v.vt = VT_BSTR;
      method.v.bstrVal = SysAllocString(L"Deflate");
      if (!method.v.bstrVal)
        fail("method allocation");
      props[n++] = method.v;
    }
    if (threads) {
      keys[n] = L"mt";
      props[n].vt = VT_UI4;
      props[n++].ulVal = threads;
    }
    if (n)
      exact(settings->SetProperties(keys, props, n), "set ZIP Deflate options");
    Source src(owner);
    src.enumerate(root, name);
    Ref<Update> cb(new Update(src));
    Ref<Output> stream(new Output(output));
    frame(47);
    exact(archive->UpdateItems(stream.get(),
                               static_cast<UInt32>(src.items.size()), cb.get()),
          "encode ZIP");
    exact(cb->error.load(), "ZIP source item failure");
    src.live();
    if (qing_machine_policy_restricts_native())
      fail("machine policy changed");
    return S_OK;
  } catch (const NotEligible &) {
    return S_FALSE;
  } catch (const Failure &e) {
    if (message && capacity)
      strncpy_s(message, capacity, e.reason, _TRUNCATE);
    return FAILED(e.hr) ? e.hr : E_FAIL;
  } catch (...) {
    if (message && capacity)
      strncpy_s(message, capacity, "ZIP writer exception", _TRUNCATE);
    return E_FAIL;
  }
}
