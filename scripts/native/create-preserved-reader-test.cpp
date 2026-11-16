// Standalone acceptance executable only. This translation unit is deliberately
// absent from build.rs; no test entry is linked into the product.
#include "../../obz-rs/native/nana/bridge.cpp"
#include <thread>
#include <cstdio>
#include <stdexcept>

// The exercised Source/Update/Input path never queries machine policy or loads
// a codec. Satisfy unrelated bridge references in this standalone executable.
extern "C" int qing_machine_policy_restricts_native() noexcept { return 1; }

static void require(bool value, const char *where) {
  if (!value) throw std::runtime_error(where);
}

int wmain(int argc, wchar_t **argv) {
  using namespace zip_create;
  try {
    require(argc == 2, "expected one synthetic fixture path");
    File parent(CreateFileW(argv[1], GENERIC_READ | FILE_WRITE_ATTRIBUTES,
                            FILE_SHARE_READ, nullptr, OPEN_EXISTING,
                            FILE_FLAG_OPEN_REPARSE_POINT, nullptr));
    require(parent.get() != INVALID_HANDLE_VALUE, "open synthetic fixture");
    FILETIME suppress{0xffffffffu, 0xffffffffu};
    win(SetFileTime(parent.get(), nullptr, &suppress, nullptr), "suppress access updates");
    BY_HANDLE_FILE_INFORMATION info{};
    win(GetFileInformationByHandle(parent.get(), &info), "fixture identity");
    require(!(info.dwFileAttributes & (FILE_ATTRIBUTE_DIRECTORY | FILE_ATTRIBUTE_REPARSE_POINT)), "ordinary fixture only");
    const uint64_t length = uint64_t(info.nFileSizeHigh) << 32 | info.nFileSizeLow;
    require(length == 4096, "4096 byte fixture required");
    HANDLE raw = nullptr;
    win(DuplicateHandle(GetCurrentProcess(), parent.get(), GetCurrentProcess(),
                        &raw, 0x100081, FALSE, 0), "readonly duplicate");
    File reader(raw);
    SetLastError(0);
    require(!SetFileTime(reader.get(), nullptr, &suppress, nullptr) &&
                GetLastError() == ERROR_ACCESS_DENIED, "child reader has no attribute write permission");
    File owner(CreateEventW(nullptr, TRUE, FALSE, nullptr));
    require(bool(owner), "test owner event");
    Source source(owner.get(), false, 1);
    Record record{L"payload.bin", L"payload.bin", nullptr,
                  uint64_t(info.nFileIndexHigh) << 32 | info.nFileIndexLow,
                  length, ticks(info.ftLastWriteTime), ticks(info.ftCreationTime),
                  ticks(info.ftLastAccessTime), info.dwFileAttributes};
    record.volume = info.dwVolumeSerialNumber;
    record.supplied_reader = reader.get();
    record.reader_mutex = std::make_shared<std::mutex>();
    source.items.push_back(std::move(record));
    Ref<Update> callback(new Update(source));
    Ref<ISequentialInStream> first, second;
    exact(callback->GetStream(0, first.put()), "first GetStream");
    exact(callback->GetStream(0, second.put()), "repeat GetStream same index");
    Ref<IInStream> a, b;
    exact(first->QueryInterface(IID_IInStream, reinterpret_cast<void **>(a.put())), "first seek interface");
    exact(second->QueryInterface(IID_IInStream, reinterpret_cast<void **>(b.put())), "second seek interface");
    auto seek = [](IInStream *stream, uint64_t offset) {
      UInt64 position = UINT64_MAX;
      exact(stream->Seek(static_cast<Int64>(offset), 0, &position), "seek fixture");
      require(position == offset, "logical position");
    };
    auto read = [&](IInStream *stream, uint64_t offset, UInt32 count) {
      BYTE bytes[67]{}; UInt32 got = UINT32_MAX;
      require(count <= sizeof(bytes), "test read bound");
      exact(stream->Read(bytes, count, &got), "read fixture");
      const UInt32 expected = static_cast<UInt32>(std::min<uint64_t>(count, offset >= length ? 0 : length - offset));
      require(got == expected, "read length");
      for (UInt32 i = 0; i < got; ++i)
        require(bytes[i] == BYTE(((offset + i) * 37 + 11) % 251), "same-record stream content");
    };
    seek(a.get(), 7); read(a.get(), 7, 5);
    seek(b.get(), 2); read(b.get(), 2, 9);
    read(a.get(), 12, 9); // Must not continue at B's physical pointer 11.
    read(b.get(), 11, 5);
    std::atomic<bool> ok{true};
    auto concurrent = [&](IInStream *stream, unsigned salt) {
      try {
        for (unsigned i = 0; i < 4096; ++i) {
          const uint64_t offset = (i * 97 + salt) % (length - 67);
          seek(stream, offset); SwitchToThread(); read(stream, offset, 67);
        }
      } catch (...) { ok = false; }
    };
    std::thread one(concurrent, a.get(), 3), two(concurrent, b.get(), 29);
    one.join(); two.join();
    require(ok, "concurrent same-record stream independence");
    seek(a.get(), 0);
    require(FAILED(a->Seek(-1, 0, nullptr)), "negative begin rejected");
    require(FAILED(a->Seek(INT64_MIN, 1, nullptr)), "negative overflow rejected");
    require(FAILED(a->Seek(0, 3, nullptr)), "unknown origin rejected");
    seek(a.get(), INT64_MAX);
    require(FAILED(a->Seek(1, 1, nullptr)), "positive overflow rejected");
    read(a.get(), INT64_MAX, 1);
    UInt64 end = 0;
    exact(a->Seek(-2, 2, &end), "end relative seek");
    require(end == length - 2, "end relative position");
    read(a.get(), length - 2, 5); read(a.get(), length, 1);
    require(callback->requested == 2 && callback->verified == 2, "both callback streams physically verified");
    win(SetEvent(owner.get()), "cancel owner");
    BYTE byte{}; UInt32 got = 99;
    require(a->Read(&byte, 1, &got) == E_ABORT && got == 0, "read after cancellation");
    require(b->Seek(0, 0, nullptr) == E_ABORT, "seek after cancellation");
    BY_HANDLE_FILE_INFORMATION after{};
    win(GetFileInformationByHandle(parent.get(), &after), "final fixture metadata");
    require(ticks(after.ftLastAccessTime) == ticks(info.ftLastAccessTime), "suppressed source access time unchanged");
    std::puts("{\"same_index_streams\":2,\"parallel_reads\":8192,\"readonly\":true,\"seek_bounds\":true,\"cancel\":true,\"atime_unchanged\":true}");
    return 0;
  } catch (const std::exception &e) { std::fprintf(stderr, "%s\n", e.what()); }
  catch (const Failure &e) { std::fprintf(stderr, "%s HRESULT=%08lx\n", e.reason, static_cast<unsigned long>(e.hr)); }
  catch (...) { std::fputs("unknown test failure\n", stderr); }
  return 1;
}
