// Create new, single-volume Deflate archives from pinned local NTFS inputs.
#include <array>
#include <map>
#include <mutex>
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
  // Logical archive timestamps are separate from physical source proof.
  HANDLE direct_root = nullptr;
  DWORD volume = 0;
  bool snapshot_time = false;
  std::array<uint64_t, 3> archive_time{}; // C, A, M
  HANDLE supplied_reader = nullptr;
  std::shared_ptr<std::mutex> reader_mutex;
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
// Order path components like the console's Windows archive enumerator. A
// separator precedes any legal filename character (e.g. a/b precedes a.rs).
// Windows uppercases individual UTF-16 units for this comparison.
wchar_t order_unit(wchar_t value) {
  if (value == L'\\')
    return 1;
  if (value >= L'a' && value <= L'z')
    return value - (L'a' - L'A');
  if (value <= 0x7f)
    return value;
  return static_cast<wchar_t>(reinterpret_cast<UINT_PTR>(
      CharUpperW(reinterpret_cast<LPWSTR>(static_cast<UINT_PTR>(value)))));
}
int path_order(std::wstring_view left, std::wstring_view right) {
  const size_t n = std::min(left.size(), right.size());
  for (size_t i = 0; i < n; ++i) {
    if (left[i] == right[i])
      continue;
    const auto a = order_unit(left[i]), b = order_unit(right[i]);
    if (a != b)
      return a < b ? -1 : 1;
  }
  return left.size() == right.size() ? 0
                                     : (left.size() < right.size() ? -1 : 1);
}
// Budget the whole *possible* representation before UpdateItems starts.
// A fresh ZIP has no prefix. Reserve the maximum EOCD comment too. ZIP32
// stores 0xffff/0xffffffff as ZIP64 sentinels, so those exact values are out.
constexpr uint64_t EndRecordsBound = 22 + UINT16_MAX;
constexpr uint64_t NotClassic = UINT64_MAX;

// Extra fields the pinned NanaZip ZipOut.cpp writes for a new, unencrypted
// item (Copy_From_UpdateItem_To_ItemOut clears Local/CentralExtra): ZIP64
// (4 + 24 central), NTFS time (4 + 32), Unix time (4 + 5) and the Info-ZIP
// Unicode name (4 + 5 + UTF-8 name). Margin is kept on top.
constexpr uint64_t FreshItemExtraBound = 128;

constexpr uint64_t next_classic_bound(uint64_t before, size_t entries,
                                      uint64_t bytes, size_t utf16_units,
                                      bool fresh_item = false) {
  constexpr uint64_t limit = UINT32_MAX - 1ull;
  // ZipAddCommon::Set_Pre_CompressionResult preselects ZIP64 at 0xf8000000.
  if (entries >= UINT16_MAX - 1u || bytes >= 0xf8000000ull ||
      utf16_units >= UINT16_MAX / 4 || before > limit)
    return NotClassic;
  // Windows UTF-8/OEM output fits four bytes per UTF-16 unit; allow the
  // trailing slash even for a file. Budget both filename fields. Each extra
  // area and the central comment have separate 16-bit length fields. A fresh
  // item from Update::GetProperty has no kpidComment (the writer stores an
  // empty comment) and only the extras listed above, whose Unicode-name part
  // is bounded by `name`. Callers without that guarantee keep the full
  // 16-bit allowance for each field.
  const uint64_t name = 4ull * (utf16_units + 1);
  const uint64_t records = 30 + 46 + 24 + 2 * name +
      (fresh_item ? 2 * (FreshItemExtraBound + name) : 3ull * UINT16_MAX);
  // Only the pinned built-in unencrypted Deflate/Store writer is eligible.
  // Its seekable streams retry Store when Deflate expands (ZipAddCommon).
  // Also reserve tentative output: DeflateEncoder::GetBlockPrice compares
  // against a stored block; even a stored block per input byte costs at
  // most six bytes of framing per byte, plus a final empty block.
  constexpr uint64_t tail = 16;
  if (records + tail > limit - before ||
      bytes > (limit - before - records - tail) / 8)
    return NotClassic;
  return before + records + tail + bytes * 8;
}
static_assert(next_classic_bound(EndRecordsBound, 1, 0, 1, true) <
              next_classic_bound(EndRecordsBound, 1, 0, 1));
// Ordinary many-small-file manifests stay on the native writer up to the
// classic entry limit; without the fresh-item guarantee they must fall back.
constexpr uint64_t small_file_budget(bool fresh, size_t count) {
  uint64_t bound = EndRecordsBound;
  for (size_t i = 0; i < count && bound != NotClassic; ++i)
    bound = next_classic_bound(bound, i, 2750, 30, fresh);
  return bound;
}
static_assert(small_file_budget(true, 20000) != NotClassic);
static_assert(small_file_budget(true, 40000) != NotClassic);
static_assert(small_file_budget(false, 20000) == NotClassic);
// Large total input still falls back before 32-bit offsets could overflow.
static_assert(next_classic_bound(EndRecordsBound, 1, 600ull * 1024 * 1024, 30, true) == NotClassic);
static_assert(next_classic_bound(UINT32_MAX - 100, 1, 0, 1, true) == NotClassic);
static_assert(next_classic_bound(EndRecordsBound, 65533, 0, 1) != NotClassic);
static_assert(next_classic_bound(EndRecordsBound, 65534, 0, 1) == NotClassic);
static_assert(next_classic_bound(EndRecordsBound, 1, UINT64_MAX, 1) ==
              NotClassic);
static_assert(next_classic_bound(EndRecordsBound, 1, 0, 20000) == NotClassic);
// Input alone fits in each case. Metadata, filename length or tentative
// Deflate output makes the otherwise identical case exceed 32-bit offsets.
static_assert(next_classic_bound(UINT32_MAX - 210000ull, 1, 0, 8) !=
              NotClassic);
static_assert(next_classic_bound(UINT32_MAX - 210000ull, 1, 0, 3000) ==
              NotClassic);
static_assert(next_classic_bound(UINT32_MAX - 196800ull, 1, 0, 1) !=
              NotClassic);
static_assert(next_classic_bound(UINT32_MAX - 196800ull, 1, 16, 1) ==
              NotClassic);

class Source {
  using Open = NTSTATUS(NTAPI *)(PHANDLE, ACCESS_MASK, POBJECT_ATTRIBUTES,
                                 PIO_STATUS_BLOCK, PLARGE_INTEGER, ULONG, ULONG,
                                 ULONG, ULONG, PVOID, ULONG);
  Open open_;
  HANDLE owner_;
  std::vector<File> pins_;
  size_t units_ = 0;
  uint64_t layout_bound_ = EndRecordsBound;
  bool classic_zip_;
  size_t snapshot_limit_;
  void append(Record r) {
    if (r.attributes & FILE_ATTRIBUTE_REPARSE_POINT)
      fail("linked input path rejected");
    if (!valid_leaf(r.name) || r.path.size() > 32767)
      throw NotEligible{};
    if (snapshot_limit_) {
      if(items.size()>=snapshot_limit_) fail("unexpected extra snapshot member");
    } else if ((units_ += r.path.size()) > 8 * 1024 * 1024 || (r.attributes & ~DWORD(0xb7)))
      throw NotEligible{};
    if (classic_zip_) {
      layout_bound_ = next_classic_bound(layout_bound_, items.size(), r.directory() ? 0 : r.size, r.path.size(), true);
      if (layout_bound_ == NotClassic) throw NotEligible{};
    } else if (!snapshot_limit_ && items.size() >= UINT16_MAX - 1u) {
      // Bounded source manifest only, not a claim about 7z physical offsets.
      throw NotEligible{};
    }
    items.emplace_back(std::move(r));
  }

public:
  std::vector<Record> items;
  bool needs_substream_size() const { return !classic_zip_; }
  explicit Source(HANDLE owner, bool classic_zip = true, size_t snapshot_limit=0) : owner_(owner), classic_zip_(classic_zip), snapshot_limit_(snapshot_limit) {
    open_ = reinterpret_cast<Open>(
        GetProcAddress(GetModuleHandleW(L"ntdll.dll"), "NtCreateFile"));
    if (!open_)
      fail("NtCreateFile unavailable");
  }
  void live() const {
    if (WaitForSingleObject(owner_, 0) != WAIT_TIMEOUT)
      fail("parent exited", E_ABORT);
  }
  File open(const Record &r) const {
    live();
    if(r.supplied_reader) {
      HANDLE duplicate=nullptr;
      win(DuplicateHandle(GetCurrentProcess(),r.supplied_reader,GetCurrentProcess(),&duplicate,0x100081,FALSE,0),"duplicate preserved reader");
      return File(duplicate);
    }
    if (r.direct_root) {
      // ReOpenFile keeps identity tied to the granted root but gives every
      // GetStream an independent file pointer (DuplicateHandle would not).
      HANDLE h=ReOpenFile(r.direct_root,0x100081,FILE_SHARE_READ,FILE_FLAG_OPEN_REPARSE_POINT);
      win(h!=INVALID_HANDLE_VALUE,"cannot reopen granted file root");
      return File(h);
    }
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
    return File(h);
  }
  static void verify(HANDLE h, const Record &r, BY_HANDLE_FILE_INFORMATION &i) {
    win(GetFileInformationByHandle(h, &i), "source file information");
    uint64_t id = (uint64_t(i.nFileIndexHigh) << 32) | i.nFileIndexLow;
    uint64_t size = (uint64_t(i.nFileSizeHigh) << 32) | i.nFileSizeLow;
    if (GetFileType(h) != FILE_TYPE_DISK ||
        (i.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT))
      fail("source is not an ordinary disk entry");
    if (((i.dwFileAttributes ^ r.attributes) & FILE_ATTRIBUTE_DIRECTORY) ||
        id != r.id || (r.volume && i.dwVolumeSerialNumber!=r.volume))
      fail("source identity changed");
    if (!r.directory() &&
        (i.dwFileAttributes != r.attributes ||
         ticks(i.ftLastWriteTime) != r.modified || size != r.size))
      fail("source size or timestamp changed");
  }
  void enumerate(HANDLE root, std::wstring name, bool direct=false) {
    wchar_t fs[32]{};
    win(GetVolumeInformationByHandleW(root, nullptr, 0, nullptr, nullptr,
                                      nullptr, fs, 32),
        "source filesystem");
    if (_wcsicmp(fs, L"NTFS") != 0)
      throw NotEligible{};
    BY_HANDLE_FILE_INFORMATION i{};
    win(GetFileInformationByHandle(root, &i), "root metadata");
    if ((i.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT) ||
        (!direct && !(i.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY)) || GetFileType(root)!=FILE_TYPE_DISK)
      fail("invalid source root");
    if (direct && !(i.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY)) {
      append({name,name,nullptr,(uint64_t(i.nFileIndexHigh)<<32)|i.nFileIndexLow,
        (uint64_t(i.nFileSizeHigh)<<32)|i.nFileSizeLow,ticks(i.ftLastWriteTime),ticks(i.ftCreationTime),ticks(i.ftLastAccessTime),i.dwFileAttributes});
      items.back().direct_root=root;items.back().volume=i.dwVolumeSerialNumber;
      return;
    }
    append({name, name, nullptr,
            (uint64_t(i.nFileIndexHigh) << 32) | i.nFileIndexLow, 0,
            ticks(i.ftLastWriteTime), ticks(i.ftCreationTime),
            ticks(i.ftLastAccessTime), i.dwFileAttributes});
    items.back().volume=i.dwVolumeSerialNumber;
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
            r.volume=i.dwVolumeSerialNumber;
            append(std::move(r));
            auto &entry = items.back();
            if (entry.directory()) {
              if (!snapshot_limit_ && pins_.size() >= 4095)
                throw NotEligible{};
              auto pin = open(entry);
              BY_HANDLE_FILE_INFORMATION actual{};
              verify(pin.get(), entry, actual);
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
      return path_order(a.path, b.path) < 0;
    });
    for (size_t j = 1; j < items.size(); ++j) {
      if (!path_order(items[j - 1].path, items[j].path))
        throw NotEligible{};
    }
  }
};


struct SnapshotOptions {
  std::vector<std::wstring> names, values;
  bool password_defined=false;
  std::wstring password;
  const BYTE *manifest=nullptr;
  size_t manifest_size=0;
  SnapshotOptions(const BYTE *data,size_t size) {
    auto take=[&](size_t n){if(n>size) fail("truncated snapshot options");const BYTE *p=data;data+=n;size-=n;return p;};
    auto number=[&](){uint32_t v;memcpy(&v,take(4),4);return v;};
    auto text=[&](){uint32_t n=number();if(n>32767)fail("snapshot option too long");std::wstring v(n,L'\0');memcpy(v.data(),take(size_t(n)*2),size_t(n)*2);if(v.find(L'\0')!=v.npos)fail("NUL snapshot option");return v;};
    const uint32_t count=number();if(count>32)fail("too many snapshot properties");
    for(uint32_t i=0;i<count;++i){auto n=text();if(n.empty())fail("empty property name");names.push_back(std::move(n));values.push_back(text());}
    const uint32_t pw=number();if(pw>1)fail("invalid password flag");password_defined=pw!=0;
    if(password_defined)password=text();
    manifest=data;manifest_size=size;
  }
  ~SnapshotOptions(){if(!password.empty())SecureZeroMemory(password.data(),password.size()*sizeof(wchar_t));}
  void apply(ISetProperties *settings) const {
    std::vector<const wchar_t*> keys;
    std::vector<PROPVARIANT> props(names.size());
    for(auto &p:props)PropVariantInit(&p);
    try {
      for(size_t i=0;i<names.size();++i){
        keys.push_back(names[i].c_str());
        // Exact console SetProperties.cpp ParseNumberString semantics.
        const auto &v=values[i];uint64_t n=0;bool numeric=!v.empty();
        for(wchar_t c:v){if(c<L'0'||c>L'9'||n>(UINT64_MAX-uint64_t(c-L'0'))/10){numeric=false;break;}n=n*10+uint64_t(c-L'0');}
        if(numeric){if(n<=UINT32_MAX){props[i].vt=VT_UI4;props[i].ulVal=static_cast<ULONG>(n);}else{props[i].vt=VT_UI8;props[i].uhVal.QuadPart=n;}}
        else {props[i].vt=VT_BSTR;props[i].bstrVal=SysAllocStringLen(v.data(),static_cast<UINT>(v.size()));if(!props[i].bstrVal)fail("snapshot property allocation",E_OUTOFMEMORY);}
      }
      exact(settings->SetProperties(keys.data(),props.data(),static_cast<UInt32>(props.size())),"snapshot compression properties");
    } catch (...) {for(auto &p:props)PropVariantClear(&p);throw;}
    for(auto &p:props)PropVariantClear(&p);
  }
};
// The parent captures times before any child lookup. Never round-trip that
// snapshot through filesystem timestamps: legitimate readers can change atime.
static void apply_time_snapshot(Source &src, const BYTE *data, size_t size,
                                const std::wstring &container, bool direct=false, bool preserve=false) {
  if (!data || size < 4 || size > 64u * 1024 * 1024)
    fail("invalid timestamp manifest");
  auto read = [&](size_t n) -> const BYTE * {
    if (n > size) fail("truncated timestamp manifest");
    const BYTE *p = data; data += n; size -= n; return p;
  };
  auto u32 = [&]() { uint32_t v; memcpy(&v, read(4), 4); return v; };
  auto u64 = [&]() { uint64_t v; memcpy(&v, read(8), 8); return v; };
  struct Times { bool directory; std::array<uint64_t,3> values; DWORD volume=0; uint64_t id=0,size=0; DWORD attributes=0; uint64_t created=0,modified=0; HANDLE reader=nullptr; };
  std::map<std::wstring,Times> snapshots;
  const uint32_t count = u32();
  // Minimum record size also bounds count before allocating anything.
  if (count > size / 30) fail("invalid timestamp manifest count");
  for (uint32_t i=0; i<count; ++i) {
    const uint32_t units=u32();
    if (!units || units>32767) fail("invalid timestamp path length");
    std::wstring path(units, L'\0');
    memcpy(path.data(), read(size_t(units)*2), size_t(units)*2);
    size_t start=0;
    for (;;) {
      const size_t end=path.find(L'\\',start);
      if (!valid_leaf(std::wstring_view(path).substr(start,end==path.npos?path.size()-start:end-start)))
        fail("invalid timestamp projection");
      if (end==path.npos) break;
      start=end+1;
    }
    const uint32_t dir=u32(); if (dir>1) fail("invalid timestamp type");
    Times t{dir!=0,{u64(),u64(),u64()}};
    if(direct) {t.volume=u32();t.id=u64();t.size=u64();t.attributes=u32();t.created=u64();t.modified=u64();}
    if(preserve) {t.reader=reinterpret_cast<HANDLE>(static_cast<uintptr_t>(u64()));if(bool(t.reader)==t.directory)fail("invalid preserved reader type");}
    if (!snapshots.emplace(std::move(path),t).second)
      fail("duplicate timestamp projection");
  }
  if (size) fail("trailing timestamp manifest");
  const std::wstring prefix=container+L"\\";
  std::vector<Record> projected;
  projected.reserve(src.items.size());
  bool root=false;
  for (auto &r:src.items) {
    if (!direct && r.path==container) {
      if (root || !r.directory()) fail("invalid snapshot container");
      root=true; continue;
    }
    if (!direct && r.path.compare(0,prefix.size(),prefix)!=0)
      fail("snapshot member escaped container");
    const std::wstring path=direct?r.path:r.path.substr(prefix.size());
    auto it=snapshots.find(path);
    if (it==snapshots.end() || it->second.directory!=r.directory())
      fail("timestamp manifest differs from verified members");
    if(direct) {
      const auto &p=it->second;
      if(r.volume!=p.volume || r.id!=p.id || r.attributes!=p.attributes ||
          r.created!=p.created || r.modified!=p.modified || (!r.directory() && r.size!=p.size))
        fail("original source differs from parent physical manifest");
    }
    if(preserve && !r.directory()) {
      BY_HANDLE_FILE_INFORMATION info{};Source::verify(it->second.reader,r,info);
      r.supplied_reader=it->second.reader;r.reader_mutex=std::make_shared<std::mutex>();
    }
    r.path=path;
    r.snapshot_time=true;
    r.archive_time=it->second.values;
    snapshots.erase(it);
    projected.push_back(std::move(r));
  }
  if ((!direct && !root) || !snapshots.empty()) fail("incomplete timestamp manifest");
  src.items=std::move(projected);
}

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
class Input final : public ComObject<IInStream>, public IStreamGetSize {
  File f_;
  const Record &record_;
  uint64_t position_ = 0;
  const Source &source_;
  std::atomic<unsigned> &verified_;
  bool checked_ = false;
  void check() {
    if (checked_) return;
    BY_HANDLE_FILE_INFORMATION info{};
    Source::verify(f_.get(), record_, info);
    checked_ = true;
    ++verified_;
  }

public:
  Input(File f, const Record &r, const Source &s, std::atomic<unsigned> &verified)
      : f_(std::move(f)), record_(r), source_(s), verified_(verified) {
    // The encoder may legitimately request no reads from an empty file.
    if (!r.size) check();
  }
  ULONG STDMETHODCALLTYPE AddRef() noexcept override { return ComObject<IInStream>::AddRef(); }
  ULONG STDMETHODCALLTYPE Release() noexcept override { return ComObject<IInStream>::Release(); }
  HRESULT STDMETHODCALLTYPE GetSize(UInt64 *size) noexcept override {
    if (!size) return E_POINTER;
    return boundary([&]() -> HRESULT {
      source_.live();
      check();
      *size = record_.size;
      return S_OK;
    });
  }
  HRESULT STDMETHODCALLTYPE QueryInterface(REFIID i,
                                           void **out) noexcept override {
    // 7z's CFolderInStream uses this size to delimit BCJ2 substreams.
    // The value is validated against the same pinned handle used for reads.
    if (i == IID_IStreamGetSize && source_.needs_substream_size()) {
      if (!out) return E_POINTER;
      *out = static_cast<IStreamGetSize *>(this);
      AddRef();
      return S_OK;
    }
    return identify(out, i == IID_IUnknown || i == IID_ISequentialInStream ||
                             i == IID_IInStream);
  }
  HRESULT STDMETHODCALLTYPE Read(void *buf, UInt32 n,
                                 UInt32 *read) noexcept override {
    if (read)
      *read = 0;
    return boundary([&]() -> HRESULT {
      source_.live();
      check();
      DWORD got = 0, requested = static_cast<DWORD>(std::min<uint64_t>(
                         position_ >= record_.size ? 0 : record_.size - position_, n));
      if (requested) {
        // Only repeated GetStream instances sharing this exact FILE_OBJECT
        // serialize their seek+read. Different records retain independent IO.
        std::unique_lock<std::mutex> lock;
        if(record_.reader_mutex) {
          lock=std::unique_lock<std::mutex>(*record_.reader_mutex);
          LARGE_INTEGER at{};at.QuadPart=static_cast<LONGLONG>(position_);
          win(SetFilePointerEx(f_.get(),at,nullptr,FILE_BEGIN),"preserved reader position");
        }
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
    return boundary([&]() -> HRESULT {
      source_.live();
      check();
      if(record_.reader_mutex) {
        if(origin>2)return STG_E_INVALIDFUNCTION;
        const uint64_t base=origin==0?0:origin==1?position_:record_.size;
        if((offset<0 && uint64_t(-(offset+1))+1>base) || (offset>=0 && uint64_t(offset)>uint64_t(INT64_MAX)-base))return STG_E_INVALIDFUNCTION;
        position_=offset<0?base-(uint64_t(-(offset+1))+1):base+uint64_t(offset);
        if(pos)*pos=position_;return S_OK;
      }
      LARGE_INTEGER d{}, result{};
      d.QuadPart = offset;
      if (!SetFilePointerEx(f_.get(), d, &result, origin))
        return HRESULT_FROM_WIN32(GetLastError());
      position_ = uint64_t(result.QuadPart);
      if (pos) *pos = position_;
      return S_OK;
    });
  }
};
class Output final : public ComObject<IOutStream> {
  HANDLE h_;
  uint64_t base_;

public:
  explicit Output(HANDLE h,uint64_t base=0) : h_(h),base_(base) {}
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
    if(origin>FILE_END || base_>INT64_MAX) return E_INVALIDARG;
    if(origin==FILE_BEGIN) {
      if(offset<0 || uint64_t(offset)>uint64_t(INT64_MAX)-base_) return E_INVALIDARG;
      d.QuadPart=offset+static_cast<int64_t>(base_);
    } else {
      LARGE_INTEGER zero{},current{};
      if(origin==FILE_CURRENT) {if(!SetFilePointerEx(h_,zero,&current,FILE_CURRENT)) return HRESULT_FROM_WIN32(GetLastError());}
      else {if(!GetFileSizeEx(h_,&current)) return HRESULT_FROM_WIN32(GetLastError());}
      if((offset<0 && uint64_t(-(offset+1))+1>uint64_t(current.QuadPart)-base_) ||
         (offset>=0 && uint64_t(offset)>uint64_t(INT64_MAX-current.QuadPart))) return E_INVALIDARG;
      d.QuadPart=current.QuadPart+offset;
      origin=FILE_BEGIN;
    }
    if (!SetFilePointerEx(h_, d, &result, origin))
      return HRESULT_FROM_WIN32(GetLastError());
    if (pos)
      *pos = result.QuadPart-static_cast<int64_t>(base_);
    return S_OK;
  }
  HRESULT STDMETHODCALLTYPE SetSize(UInt64 n) noexcept override {
    FILE_END_OF_FILE_INFO end{};
    if(n>uint64_t(INT64_MAX)-base_)return E_INVALIDARG;
    end.EndOfFile.QuadPart = n+base_;
    return SetFileInformationByHandle(h_, FileEndOfFileInfo, &end, sizeof(end))
               ? S_OK
               : HRESULT_FROM_WIN32(GetLastError());
  }
};
class Update final : public ComObject<IArchiveUpdateCallback>, public ICryptoGetTextPassword2 {
  const SnapshotOptions *options_;
  Source &src_;
  std::mutex progress_;
  UInt64 total_ = 0;
  BYTE last_ = 0;

public:
  std::atomic<HRESULT> error{S_OK};
  std::atomic<unsigned> requested{0}, verified{0};
  explicit Update(Source &s,const SnapshotOptions *options=nullptr) : options_(options), src_(s) {}
  ULONG STDMETHODCALLTYPE AddRef() noexcept override {return ComObject<IArchiveUpdateCallback>::AddRef();}
  ULONG STDMETHODCALLTYPE Release() noexcept override {return ComObject<IArchiveUpdateCallback>::Release();}
  HRESULT STDMETHODCALLTYPE CryptoGetTextPassword2(Int32 *defined,BSTR *password) noexcept override {
    if(!defined||!password)return E_POINTER;*defined=0;*password=nullptr;
    if(options_ && options_->password_defined){*password=SysAllocStringLen(options_->password.data(),static_cast<UINT>(options_->password.size()));if(!*password)return E_OUTOFMEMORY;*defined=1;}
    return S_OK;
  }
  HRESULT STDMETHODCALLTYPE QueryInterface(REFIID i,
                                           void **out) noexcept override {
    if(i==IID_ICryptoGetTextPassword2 && options_){if(!out)return E_POINTER;*out=static_cast<ICryptoGetTextPassword2*>(this);AddRef();return S_OK;}
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
      const auto value = r.snapshot_time
          ? r.archive_time[id == kpidCTime ? 0 : (id == kpidATime ? 1 : 2)]
          : (id == kpidMTime ? r.modified : (id == kpidCTime ? r.created : r.accessed));
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
      // Take the same deny-write/delete pin at the callback boundary. Only the
      // metadata check moves to the codec reader, before its first read/seek.
      *out = new Input(src_.open(r), r, src_, verified);
      ++requested;
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

static HRESULT qz_create_archive(const wchar_t *runtime, HANDLE owner,
                                 HANDLE root, const wchar_t *name,
                                 HANDLE output, DWORD level, DWORD threads,
                                 DWORD explicit_method, char *message,
                                 size_t capacity, bool seven, DWORD dictionary, DWORD solid, DWORD times=0, uint64_t *latest=nullptr, const BYTE *snapshot=nullptr, size_t snapshot_size=0, const zip_create::SnapshotOptions *snapshot_options=nullptr, uint64_t output_base=0, const std::vector<std::pair<HANDLE,std::wstring>> *direct_roots=nullptr, bool preserve=false) noexcept {
  using namespace zip_create;
  bool started=false;
  try {
    if (!runtime || !owner || !root || !name || !output ||
        (level != UINT32_MAX && (level < (snapshot_options?0u:1u) || level > (snapshot_options?49u:9u))) || (!snapshot_options && threads > 64) ||
        explicit_method > 1 || solid > 2 || (dictionary && (dictionary < 4096 || dictionary > (1u<<30))))
      fail("invalid ZIP request");
    if ((times & ~127u) || (times&3)==3 || ((times>>2)&3)==3 || ((times>>4)&3)==3 || ((times&64)&&!latest)) fail("invalid timestamp request");
    if (qing_machine_policy_restricts_native())
      fail("machine policy rejects native route");
    CPINFO oem{};
    // The ZIP layout budget assumes at most four filename bytes per UTF-16
    // unit. UTF-8 satisfies this; qualify the machine's actual OEM page too.
    if (!GetCPInfo(CP_OEMCP, &oem) || oem.MaxCharSize > 4)
      throw NotEligible{};
    Runtime dll;
    dll.init(runtime);
    // Keep records and callback counters alive even if the handler retains
    // input streams until its own destruction (including exception paths).
    uint32_t snapshot_count=0;
    if(snapshot_options) {
      if(snapshot_size<4 || snapshot_size>64u*1024*1024) fail("invalid snapshot manifest size");
      memcpy(&snapshot_count,snapshot,4);
      if(snapshot_count>(snapshot_size-4)/30) fail("invalid snapshot record count");
    }
    Source src(owner, !seven && !snapshot_options, snapshot_options ? size_t(snapshot_count)+1 : 0);
    Ref<Update> cb(new Update(src,snapshot_options));
    std::vector<File> empty_pins;
    Ref<IOutArchive> archive;
    dll.handler(seven ? L"7z" : L"zip", seven ? "7z" : "Zip", archive.put(), IID_IOutArchive);
    Ref<ISetProperties> settings;
    exact(archive->QueryInterface(IID_ISetProperties,
                                  reinterpret_cast<void **>(settings.put())),
          "ZIP options interface");
    if(snapshot_options) {snapshot_options->apply(settings.get());} else {
    const wchar_t *keys[8]{};
    PROPVARIANT props[8]{};
    unsigned n = 0;
    if (level != UINT32_MAX) {
      keys[n] = L"x";
      props[n].vt = VT_UI4;
      props[n++].ulVal = level;
    }
    Variant method;
    if (explicit_method) {
      keys[n] = seven ? L"0" : L"m";
      method.v.vt = VT_BSTR;
      method.v.bstrVal = SysAllocString(seven ? L"LZMA2" : L"Deflate");
      if (!method.v.bstrVal)
        fail("method allocation");
      props[n++] = method.v;
    }
    if (threads) {
      keys[n] = L"mt";
      props[n].vt = VT_UI4;
      props[n++].ulVal = threads;
    }
    Variant dictionary_value;
    if (seven && dictionary) {
      // MethodProps::PROPVARIANT_to_DictSize treats VT_UI4 as log2(bits),
      // whereas the public option is bytes. Match console -md=<bytes>b.
      dictionary_value.v.vt=VT_BSTR;
      const auto bytes=std::to_wstring(dictionary)+L"b";
      dictionary_value.v.bstrVal=SysAllocString(bytes.c_str());
      if (!dictionary_value.v.bstrVal) fail("dictionary allocation");
      keys[n]=L"0d";props[n++]=dictionary_value.v;
    }
    if (seven && solid) {
      keys[n]=L"s";props[n].vt=VT_BOOL;props[n++].boolVal=solid==2 ? VARIANT_TRUE : VARIANT_FALSE;
    }
    const wchar_t *time_keys[]={L"tm",L"tc",L"ta"};
    for(unsigned i=0;i<3;++i) {
      const auto value=(times>>(i*2))&3;
      if(value) { keys[n]=time_keys[i];props[n].vt=VT_BOOL;props[n++].boolVal=value==2?VARIANT_TRUE:VARIANT_FALSE; }
    }
    if (n) exact(settings->SetProperties(keys, props, n), "set archive compression options");
    }
    if(direct_roots) {
      for(const auto &r:*direct_roots) src.enumerate(r.first,r.second,true);
      std::sort(src.items.begin(),src.items.end(),[](const Record &a,const Record &b){return path_order(a.path,b.path)<0;});
      for(size_t i=1;i<src.items.size();++i) if(!path_order(src.items[i-1].path,src.items[i].path)) fail("duplicate direct source projection");
    } else src.enumerate(root, name);
    if (snapshot) apply_time_snapshot(src,snapshot,snapshot_size,name,direct_roots!=nullptr,preserve);
    // 7z stores zero-byte entries without requesting GetStream. Validate and
    // retain their deny-write/delete handles through UpdateItems as well.
    if (seven) for (const auto &r : src.items) {
      if (!r.directory() && !r.size) {
        auto pin = src.open(r);
        BY_HANDLE_FILE_INFORMATION info{};
        Source::verify(pin.get(), r, info);
        empty_pins.push_back(std::move(pin));
      }
    }
    Ref<Output> stream(new Output(output,output_base));
    exact(stream->Seek(0,FILE_BEGIN,nullptr),"archive output position");
    frame(47);
    started=true;
    exact(archive->UpdateItems(stream.get(),
                               static_cast<UInt32>(src.items.size()), cb.get()),
          "encode ZIP");
    exact(cb->error.load(), "ZIP source item failure");
    if (cb->requested.load() != cb->verified.load())
      fail("ZIP source stream was not validated");
    src.live();
    if (qing_machine_policy_restricts_native())
      fail("machine policy changed");
    if (times&64) {
      *latest=0;
      for (const auto &record:src.items) *latest=std::max(*latest,record.snapshot_time ? record.archive_time[2] : record.modified);
    }
    return S_OK;
  } catch (const NotEligible &) {
    // A started encoder can never become an unsupported/fallback receipt.
    return started ? E_FAIL : S_FALSE;
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

extern "C" HRESULT qz_create_zip(const wchar_t *runtime, HANDLE owner, HANDLE root,
 const wchar_t *name, HANDLE output, DWORD level, DWORD threads, DWORD explicit_method,
 char *message, size_t capacity) noexcept {
 return qz_create_archive(runtime,owner,root,name,output,level,threads,explicit_method,message,capacity,false,0,0);
}
extern "C" HRESULT qz_create_7z(const wchar_t *runtime, HANDLE owner, HANDLE root,
 const wchar_t *name, HANDLE output, DWORD level, DWORD threads, DWORD explicit_method,
 DWORD dictionary, DWORD solid, char *message, size_t capacity) noexcept {
 return qz_create_archive(runtime,owner,root,name,output,level,threads,explicit_method,message,capacity,true,dictionary,solid);
}

extern "C" HRESULT qz_create_timed(const wchar_t *runtime,HANDLE owner,HANDLE root,const wchar_t *name,HANDLE output,
 DWORD level,DWORD threads,DWORD explicit_method,DWORD dictionary,DWORD solid,DWORD seven,DWORD times,uint64_t *latest,char *message,size_t capacity) noexcept {
 if(seven>1) return E_INVALIDARG;
 return qz_create_archive(runtime,owner,root,name,output,level,threads,explicit_method,message,capacity,seven!=0,dictionary,solid,times,latest);
}

extern "C" HRESULT qz_create_snapshot(const wchar_t *runtime,HANDLE owner,HANDLE root,const wchar_t *name,HANDLE output,
 DWORD level,DWORD threads,DWORD explicit_method,DWORD dictionary,DWORD solid,DWORD seven,DWORD times,
 const BYTE *snapshot,size_t snapshot_size,uint64_t output_base,uint64_t *latest,char *message,size_t capacity) noexcept {
 if(seven>1 || !snapshot) return E_INVALIDARG;
 try {
   zip_create::SnapshotOptions options(snapshot,snapshot_size);
   return qz_create_archive(runtime,owner,root,name,output,level,threads,explicit_method,message,capacity,seven!=0,dictionary,solid,times,latest,options.manifest,options.manifest_size,&options,output_base);
 } catch(const Failure &e) {if(message&&capacity)strncpy_s(message,capacity,e.reason,_TRUNCATE);return FAILED(e.hr)?e.hr:E_FAIL;}
 catch(...) {return E_FAIL;}
}

// Direct-source protocol: independently bounded roots followed by the existing
// property/password section and a physical+logical manifest generated by parent.
extern "C" HRESULT qz_create_direct(const wchar_t *runtime,HANDLE owner,HANDLE root,const wchar_t *name,HANDLE output,
 DWORD level,DWORD threads,DWORD explicit_method,DWORD dictionary,DWORD solid,DWORD seven,DWORD times,
 const BYTE *snapshot,size_t snapshot_size,uint64_t output_base,DWORD preserve,uint64_t *latest,char *message,size_t capacity) noexcept {
 if(seven>1 || preserve>1 || !snapshot || snapshot_size>64u*1024*1024) return E_INVALIDARG;
 try {
   auto take=[&](size_t n){if(n>snapshot_size)fail("truncated direct source roots");const BYTE *p=snapshot;snapshot+=n;snapshot_size-=n;return p;};
   auto u32=[&](){uint32_t v;memcpy(&v,take(4),4);return v;};
   uint32_t count=u32();if(!count || count>snapshot_size/14)fail("invalid source root count");
   std::vector<std::pair<HANDLE,std::wstring>> roots;roots.reserve(count);
   for(uint32_t i=0;i<count;++i) {
     uint64_t handle;memcpy(&handle,take(8),8);uint32_t units=u32();
     if(!units || units>255)fail("invalid source root projection");
     std::wstring name(units,L'\0');memcpy(name.data(),take(size_t(units)*2),size_t(units)*2);
     if(!zip_create::valid_leaf(name) || !handle)fail("invalid source root");
     roots.emplace_back(reinterpret_cast<HANDLE>(static_cast<uintptr_t>(handle)),std::move(name));
   }
   zip_create::SnapshotOptions options(snapshot,snapshot_size);
   return qz_create_archive(runtime,owner,root,name,output,level,threads,explicit_method,message,capacity,seven!=0,dictionary,solid,times,latest,options.manifest,options.manifest_size,&options,output_base,&roots,preserve!=0);
 } catch(const Failure &e) {if(message&&capacity)strncpy_s(message,capacity,e.reason,_TRUNCATE);return FAILED(e.hr)?e.hr:E_FAIL;}
 catch(...) {return E_FAIL;}
}

// Parent-side traversal helpers. Relative, non-reparse opens never re-resolve
// an ancestor pathname; enumeration returns one bounded page of actual names.
extern "C" HRESULT qz_create_input_open(HANDLE parent,const wchar_t *name,DWORD access,DWORD share,HANDLE *result) noexcept {
 if(!parent||!name||!result)return E_INVALIDARG;*result=nullptr;
 try {
   std::wstring_view leaf(name);if(!zip_create::valid_leaf(leaf))fail("invalid source leaf");
   using Open=NTSTATUS(NTAPI *)(PHANDLE,ACCESS_MASK,POBJECT_ATTRIBUTES,PIO_STATUS_BLOCK,PLARGE_INTEGER,ULONG,ULONG,ULONG,ULONG,PVOID,ULONG);
   auto open=reinterpret_cast<Open>(GetProcAddress(GetModuleHandleW(L"ntdll.dll"),"NtCreateFile"));if(!open)return E_NOTIMPL;
   UNICODE_STRING text{};text.Buffer=const_cast<wchar_t *>(name);text.Length=text.MaximumLength=static_cast<USHORT>(leaf.size()*2);
   OBJECT_ATTRIBUTES object{};object.Length=sizeof(object);object.RootDirectory=parent;object.ObjectName=&text;object.Attributes=0x1000;
   IO_STATUS_BLOCK io{};HANDLE handle=nullptr;
   NTSTATUS status=open(&handle,access|SYNCHRONIZE,&object,&io,nullptr,0,share,FILE_OPEN,0x00200000|0x20|0x4000,nullptr,0);
   if(status<0) {using Convert=ULONG(WINAPI *)(NTSTATUS);auto convert=reinterpret_cast<Convert>(GetProcAddress(GetModuleHandleW(L"ntdll.dll"),"RtlNtStatusToDosError"));return convert?HRESULT_FROM_WIN32(convert(status)):HRESULT_FROM_NT(status);}
   *result=handle;return S_OK;
 }catch(const Failure &e){return FAILED(e.hr)?e.hr:E_FAIL;}catch(...){return E_FAIL;}
}
extern "C" HRESULT qz_create_input_names(HANDLE directory,DWORD restart,BYTE *names,DWORD capacity,DWORD *used) noexcept {
 if(!directory||!names||!used||capacity<65536||restart>1)return E_INVALIDARG;*used=0;
 try {
   alignas(8) std::array<BYTE,65536> buffer{};
   if(!GetFileInformationByHandleEx(directory,restart?FileIdBothDirectoryRestartInfo:FileIdBothDirectoryInfo,buffer.data(),DWORD(buffer.size()))) {
     DWORD error=GetLastError();return error==ERROR_NO_MORE_FILES?S_FALSE:HRESULT_FROM_WIN32(error);
   }
   size_t offset=0;
   for(;;) {
     constexpr size_t header=offsetof(FILE_ID_BOTH_DIR_INFO,FileName);
     if(offset>buffer.size()-header)fail("invalid directory page");
     auto &e=*reinterpret_cast<const FILE_ID_BOTH_DIR_INFO *>(buffer.data()+offset);
     if(e.FileNameLength%2||e.FileNameLength>buffer.size()-offset-header)fail("invalid directory name");
     std::wstring_view name(e.FileName,e.FileNameLength/2);
     if(name!=L"."&&name!=L"..") {
       if(!zip_create::valid_leaf(name))fail("invalid source leaf");
       if(e.FileNameLength>capacity-*used || capacity-*used-e.FileNameLength<4)fail("directory page exceeds bound");
       DWORD units=e.FileNameLength/2;memcpy(names+*used,&units,4);memcpy(names+*used+4,name.data(),e.FileNameLength);*used+=4+e.FileNameLength;
     }
     if(!e.NextEntryOffset)break;
     if(e.NextEntryOffset<header||e.NextEntryOffset%8||e.NextEntryOffset>buffer.size()-offset)fail("invalid directory offset");
     offset+=e.NextEntryOffset;
   }
   return S_OK;
 }catch(const Failure &e){return FAILED(e.hr)?e.hr:E_FAIL;}catch(...){return E_FAIL;}
}
