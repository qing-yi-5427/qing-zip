// Native one-job child. A separate private input frame carries the optional password.
// The decoder reads pinned volume handles and writes private output handles.
#include "bridge_contract.h"
#include <algorithm>
#include <array>
#include <atomic>
#include <cstdint>
#include <cstring>
#include <mutex>
#include <stdexcept>
#include <string>
#include <vector>

namespace {
constexpr size_t frame_limit = 16 * 1024 * 1024;
constexpr uint32_t no_index = UINT32_MAX;
using Bytes = std::vector<uint8_t>;

void transfer(HANDLE pipe, void *data, size_t length, bool writing) {
  auto *bytes = static_cast<uint8_t *>(data);
  while (length) {
    DWORD count = 0;
    DWORD requested = static_cast<DWORD>(std::min<size_t>(length, 64 * 1024));
    BOOL ok = writing ? WriteFile(pipe, bytes, requested, &count, nullptr)
                      : ReadFile(pipe, bytes, requested, &count, nullptr);
    if (!ok || !count || count > requested) throw std::runtime_error("worker control pipe closed");
    length -= count;
    bytes += count;
  }
}
void append(Bytes &bytes, const void *src, size_t n) {
  if (n > frame_limit - bytes.size()) throw std::runtime_error("worker frame limit exceeded");
  const auto *p = static_cast<const uint8_t *>(src);
  bytes.insert(bytes.end(), p, p + n);
}
template<class T> void put(Bytes &bytes, T value) { append(bytes, &value, sizeof(value)); }
struct Reader {
  Bytes bytes;
  size_t at = 0;
  void take(void *target, size_t n) {
    if (n > bytes.size() - at) throw std::runtime_error("truncated worker request");
    memcpy(target, bytes.data() + at, n); at += n;
  }
  template<class T> T get() { T value{}; take(&value, sizeof(value)); return value; }
  std::wstring wide() {
    auto length = get<uint32_t>();
    if (length > 32767) throw std::runtime_error("invalid UTF-16 field length");
    std::wstring result(length, L'\0'); take(result.data(), length * 2); return result;
  }
  void end() const { if (at != bytes.size()) throw std::runtime_error("unexpected control data"); }
};
struct Channel {
  HANDLE input = GetStdHandle(STD_INPUT_HANDLE);
  HANDLE output = GetStdHandle(STD_OUTPUT_HANDLE);
  std::mutex write_lock;
  Reader read(uint8_t expected) {
    uint8_t kind; uint32_t length;
    transfer(input, &kind, 1, false); transfer(input, &length, 4, false);
    if (kind != expected || length > frame_limit) throw std::runtime_error("invalid worker frame");
    Reader r; r.bytes.resize(length); transfer(input, r.bytes.data(), length, false); return r;
  }
  void send(uint8_t kind, Bytes &bytes) {
    std::lock_guard<std::mutex> lock(write_lock);
    auto length = static_cast<uint32_t>(bytes.size());
    transfer(output, &kind, 1, true); transfer(output, &length, 4, true);
    transfer(output, bytes.data(), length, true);
  }
};
struct Worker {
  Channel channel;
  std::array<uint8_t, 16> nonce{};
  // All duplicated handles are closed by the kernel on this one-job process's
  // exit, even if a codec throws. Output streams are explicitly closed earlier.
  std::vector<HANDLE> sources, outputs;
  std::vector<std::wstring> leaves;
  std::vector<QzSelectedVolume> volumes;
  std::vector<uint8_t> selected;
  std::mutex source_lock;
  std::atomic<bool> failed{false}, header_notified{false};
  std::atomic<int32_t> operation_error{-1};
  std::atomic<ULONGLONG> last_progress{0};
  HANDLE parent{};
  std::vector<wchar_t> password;
  ~Worker() {
    if (!password.empty()) SecureZeroMemory(password.data(), password.size() * sizeof(wchar_t));
  }
  Bytes message() const { return Bytes(nonce.begin(), nonce.end()); }
  void match_nonce(Reader &r) {
    std::array<uint8_t, 16> received{}; r.take(received.data(), received.size());
    if (received != nonce) throw std::runtime_error("worker job identity mismatch");
  }
  void start(const wchar_t *runtime) {
    auto r = channel.read(1); r.take(nonce.data(), nonce.size());
    if (r.wide() != runtime) throw std::runtime_error("runtime directory mismatch");
    parent = reinterpret_cast<HANDLE>(r.get<uint64_t>());
    if (WaitForSingleObject(parent, 0) != WAIT_TIMEOUT) throw std::runtime_error("parent process unavailable");
    auto count = r.get<uint32_t>();
    if (!count || count > 1024) throw std::runtime_error("invalid number of volumes");
    sources.reserve(count); leaves.reserve(count); volumes.reserve(count);
    for (uint32_t i = 0; i < count; ++i) {
      auto handle = reinterpret_cast<HANDLE>(r.get<uint64_t>());
      auto size = r.get<uint64_t>(); leaves.push_back(r.wide());
      BY_HANDLE_FILE_INFORMATION info{};
      if (GetFileType(handle) != FILE_TYPE_DISK || !GetFileInformationByHandle(handle, &info) ||
          (info.dwFileAttributes & (FILE_ATTRIBUTE_DIRECTORY | FILE_ATTRIBUTE_REPARSE_POINT)) ||
          (((uint64_t(info.nFileSizeHigh) << 32) | info.nFileSizeLow) != size))
        throw std::runtime_error("invalid pinned source handle");
      sources.push_back(handle);
      volumes.push_back({leaves.back().c_str(), uint32_t(leaves.back().size()), 0, size});
    }
    auto units = r.get<uint32_t>();
    if (units > 32767) throw std::runtime_error("password input length exceeds limit");
    r.end();
    if (units) {
      uint8_t kind; uint32_t length;
      transfer(channel.input, &kind, 1, false); transfer(channel.input, &length, 4, false);
      if (kind != 6 || length != units * sizeof(wchar_t)) throw std::runtime_error("invalid password input");
      password.resize(units);
      transfer(channel.input, password.data(), length, false);
    }
  }
};
template<class Fn> HRESULT guarded(Worker &w, Fn &&f) noexcept {
  try { return f(); } catch (...) { w.failed = true; return E_FAIL; }
}
HRESULT __stdcall read_at(void *context, uint32_t index, uint64_t offset, void *buffer, uint32_t size, uint32_t *got) {
  auto &w = *static_cast<Worker *>(context); *got = 0;
  return guarded(w, [&]() -> HRESULT {
    if (index >= w.sources.size() || offset > INT64_MAX || offset > w.volumes[index].fixed_size ||
        size > w.volumes[index].fixed_size - offset) return E_INVALIDARG;
    std::lock_guard<std::mutex> lock(w.source_lock);
    LARGE_INTEGER pos{}; pos.QuadPart = static_cast<LONGLONG>(offset); DWORD n = 0;
    if (!SetFilePointerEx(w.sources[index], pos, nullptr, FILE_BEGIN) ||
        !ReadFile(w.sources[index], buffer, size, &n, nullptr)) return HRESULT_FROM_WIN32(GetLastError());
    *got = n; return S_OK;
  });
}
HRESULT __stdcall accept_manifest(void *context, const QzSelectedItem *items, uint32_t count, uint8_t *ack) {
  auto &w = *static_cast<Worker *>(context);
  return guarded(w, [&]() -> HRESULT {
    Bytes msg = w.message(); put(msg, count);
    for (uint32_t i = 0; i < count; ++i) {
      const auto &e = items[i]; if (e.index != i) return E_FAIL;
      put(msg, e.name_units); append(msg, e.name, size_t(e.name_units) * 2);
      put(msg, e.size); put(msg, e.is_dir); put(msg, e.attrs_present); put(msg, e.attrs);
      put(msg, e.crc_present); put(msg, e.crc);
      for (auto t : {e.ctime, e.atime, e.mtime}) { put(msg, t.present); put(msg, t.filetime_100ns); }
    }
    w.channel.send(2, msg);
    auto r = w.channel.read(3); w.match_nonce(r);
    if (r.get<uint32_t>() != count) return E_FAIL;
    w.outputs.assign(count, nullptr); w.selected.resize(count);
    for (uint32_t i = 0; i < count; ++i) {
      auto selected = r.get<uint8_t>(); auto handle = reinterpret_cast<HANDLE>(r.get<uint64_t>());
      if (selected > 1 || bool(handle) != bool(selected && !items[i].is_dir)) return E_FAIL;
      if (handle) {
        BY_HANDLE_FILE_INFORMATION info{};
        if (GetFileType(handle) != FILE_TYPE_DISK || !GetFileInformationByHandle(handle, &info) ||
            info.nNumberOfLinks != 1 || info.nFileSizeHigh || info.nFileSizeLow ||
            (info.dwFileAttributes & (FILE_ATTRIBUTE_DIRECTORY | FILE_ATTRIBUTE_REPARSE_POINT))) return E_FAIL;
      }
      w.outputs[i] = handle; w.selected[i] = selected; ack[i] = selected;
    }
    r.end(); return S_OK;
  });
}
HRESULT __stdcall prepared(void *) { return S_OK; } // Parent prepared directories before granting capabilities.
HRESULT __stdcall open_output(void *context, uint32_t index, uint64_t *token) {
  auto &w = *static_cast<Worker *>(context);
  if (index >= w.outputs.size() || !w.outputs[index]) return E_FAIL;
  *token = index; return S_OK;
}
HRESULT __stdcall write_output(void *context, uint64_t token, const void *buffer, uint32_t size, uint32_t *processed) {
  auto &w = *static_cast<Worker *>(context); *processed = 0; DWORD n = 0;
  if (token >= w.outputs.size() || !w.outputs[size_t(token)]) return E_FAIL;
  if (!WriteFile(w.outputs[size_t(token)], buffer, size, &n, nullptr)) return HRESULT_FROM_WIN32(GetLastError());
  *processed = n; return S_OK;
}
HRESULT __stdcall close_output(void *context, uint64_t token) {
  auto &w = *static_cast<Worker *>(context);
  if (token >= w.outputs.size() || !w.outputs[size_t(token)]) return E_FAIL;
  HANDLE h = w.outputs[size_t(token)]; w.outputs[size_t(token)] = nullptr;
  return CloseHandle(h) ? S_OK : HRESULT_FROM_WIN32(GetLastError());
}
int32_t __stdcall cancelled(void *p) {
  auto &w = *static_cast<Worker *>(p);
  return w.failed || WaitForSingleObject(w.parent, 0) != WAIT_TIMEOUT;
}
void __stdcall progress(void *p, uint64_t decoded, uint64_t total) {
  auto &w = *static_cast<Worker *>(p); ULONGLONG now = GetTickCount64(), previous = w.last_progress.load();
  if (now - previous < 250 || !w.last_progress.compare_exchange_strong(previous, now)) return;
  guarded(w, [&]() -> HRESULT {
    Bytes msg = w.message(); put(msg, decoded); put(msg, total); w.channel.send(4, msg); return S_OK;
  });
}
HRESULT __stdcall observe(void *p, uint32_t event, uint32_t, int32_t, int32_t result, uint64_t) {
  auto &w = *static_cast<Worker *>(p);
  if (event == QZ_TRACE_ENCRYPTED_HEADER && !w.header_notified.exchange(true)) {
    return guarded(w, [&]() -> HRESULT {
      auto msg = w.message(); w.channel.send(7, msg); return S_OK;
    });
  }
  if ((event == 22 || event == 23) && result != 0) w.operation_error = result;
  return S_OK;
}
}

extern "C" int qing_selected_worker_main(const wchar_t *runtime) noexcept {
  try {
    Worker w; w.start(runtime);
    QzSelectedHost host{1, sizeof(QzSelectedHost), &w, read_at, accept_manifest, prepared,
                        open_output, write_output, close_output, cancelled, progress, observe};
    QzSelectedRequest req{1, sizeof(QzSelectedRequest), runtime, w.volumes.data(),
                          uint32_t(w.volumes.size()), 0, &host};
    QzSelectedReceipt result{}; HRESULT hr = qz_extract_selected_with_password(&req, &result, w.password.data(), static_cast<uint32_t>(w.password.size()));
    Bytes msg = w.message(); put(msg, uint32_t(hr)); put(msg, result.outcome); put(msg, result.item_count);
    put(msg, result.completion_count); put(msg, result.item_errors); put(msg, result.block_errors);
    put(msg, result.streams_open); put(msg, result.decoded_bytes); put(msg, result.selected_bytes);
    put(msg, w.operation_error.load());
    auto reason_length = uint32_t(strnlen_s(result.reason, sizeof(result.reason)));
    put(msg, reason_length); append(msg, result.reason, reason_length); w.channel.send(5, msg);
    return hr == S_OK && (result.outcome == QZ_SELECTED_DECODED_PRIVATE || result.outcome == QZ_SELECTED_UNSUPPORTED_LAYOUT)
        && !w.failed ? 0 : 2;
  } catch (...) { return 3; }
}
