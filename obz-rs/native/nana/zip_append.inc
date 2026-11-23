// Default ZIP Add using only pinned sources and an owned seekable output.
namespace zip_append {
struct Root { HANDLE source; const wchar_t *leaf; };

struct Item {
  std::wstring path;
  UInt32 existing = UINT32_MAX;
  UInt32 incoming = UINT32_MAX;
  bool directory = false;
};

bool property_flag(IInArchive *archive, UInt32 index, PROPID property) {
  Variant value;
  exact(archive->GetProperty(index, property, &value.v), "read ZIP member flag");
  if (value.v.vt == VT_EMPTY) return false;
  if (value.v.vt != VT_BOOL) fail("invalid ZIP member flag");
  return value.v.boolVal == VARIANT_TRUE;
}

std::vector<Item> plan(IInArchive *archive, UInt32 count, const zip_create::Source &source) {
  using zip_create::NotEligible;
  using zip_create::path_order;
  std::vector<Item> names;
  names.reserve(count + source.items.size());
  size_t name_units = 0;
  for (UInt32 i = 0; i < count; ++i) {
    if (property_flag(archive, i, kpidEncrypted)) throw NotEligible{};
    const bool directory = property_flag(archive, i, kpidIsDir);
    Variant value;
    exact(archive->GetProperty(i, kpidPath, &value.v), "read original ZIP member path");
    auto path = text(value.v);
    std::replace(path.begin(), path.end(), L'/', L'\\');
    if (directory && path.ends_with(L"\\")) path.pop_back();
    if (path.empty() || (name_units += path.size()) > MAX_MANIFEST_UNITS) throw NotEligible{};
    names.push_back({std::move(path), i, UINT32_MAX, directory});
  }
  for (UInt32 i = 0; i < source.items.size(); ++i) {
    const auto &entry = source.items[i];
    names.push_back({entry.path, UINT32_MAX, i, entry.directory()});
  }
  std::sort(names.begin(), names.end(), [](const Item &a, const Item &b) {
    const int order = path_order(a.path, b.path);
    return order ? order < 0 : a.existing != UINT32_MAX && b.existing == UINT32_MAX;
  });
  std::vector<Item> result;
  result.reserve(names.size());
  for (size_t i = 0; i < names.size(); ++i) {
    auto member = std::move(names[i]);
    if (i + 1 < names.size() && !path_order(member.path, names[i + 1].path)) {
      const auto &replacement = names[++i];
      if (member.existing == UINT32_MAX || replacement.incoming == UINT32_MAX ||
          member.directory != replacement.directory ||
          (i + 1 < names.size() && !path_order(member.path, names[i + 1].path)))
        throw NotEligible{};
      member.incoming = replacement.incoming;
    }
    if (!result.empty() && !result.back().directory) {
      const auto &previous = result.back().path;
      const auto current = std::wstring_view(member.path);
      if (current.size() > previous.size() && current[previous.size()] == L'\\' &&
          !path_order(current.substr(0, previous.size()), previous)) throw NotEligible{};
    }
    result.push_back(std::move(member));
  }
  return result;
}

class Update final : public zip_create::ComObject<IArchiveUpdateCallback> {
  const std::vector<Item> &plan_;
  const Ref<zip_create::Update> incoming_;
public:
  Update(const std::vector<Item> &plan, zip_create::Source &incoming)
      : plan_(plan), incoming_(new zip_create::Update(incoming)) {}
  HRESULT STDMETHODCALLTYPE QueryInterface(REFIID i, void **out) noexcept override {
    return identify(out, i == IID_IUnknown || i == IID_IProgress || i == IID_IArchiveUpdateCallback);
  }
  HRESULT STDMETHODCALLTYPE GetUpdateItemInfo(UInt32 i, Int32 *data, Int32 *props, UInt32 *old) noexcept override {
    if (i >= plan_.size()) return E_INVALIDARG;
    const auto &item = plan_[i];
    if (data) *data = item.incoming != UINT32_MAX;
    if (props) *props = item.incoming != UINT32_MAX;
    if (old) *old = item.existing;
    return S_OK;
  }
  HRESULT STDMETHODCALLTYPE GetProperty(UInt32 i, PROPID id, PROPVARIANT *value) noexcept override {
    if (i >= plan_.size() || !value) return E_INVALIDARG;
    if (plan_[i].incoming != UINT32_MAX) return incoming_->GetProperty(plan_[i].incoming, id, value);
    PropVariantInit(value);
    return S_OK;
  }
  HRESULT STDMETHODCALLTYPE GetStream(UInt32 i, ISequentialInStream **out) noexcept override {
    return i < plan_.size() && plan_[i].incoming != UINT32_MAX
        ? incoming_->GetStream(plan_[i].incoming, out) : E_FAIL;
  }
  HRESULT STDMETHODCALLTYPE SetOperationResult(Int32 result) noexcept override { return incoming_->SetOperationResult(result); }
  HRESULT STDMETHODCALLTYPE SetCompleted(const UInt64 *value) noexcept override { return incoming_->SetCompleted(value); }
  HRESULT STDMETHODCALLTYPE SetTotal(UInt64 value) noexcept override { return incoming_->SetTotal(value); }
  void verify() {
    exact(incoming_->error.load(), "ZIP member update failed");
    if (incoming_->requested != incoming_->verified) fail("ZIP source stream was not verified");
  }
};
}

extern "C" HRESULT qz_append_zip(const wchar_t *runtime_dir, HANDLE owner, HANDLE original,
    HANDLE output, UInt32 expected_count, const zip_append::Root *roots, UInt32 root_count,
    char *error, size_t capacity) noexcept {
  using namespace zip_create;
  bool started = false;
  try {
    if (!owner || !original || !output || !roots || !root_count || root_count > 4096 || expected_count >= UINT16_MAX)
      fail("invalid ZIP update request");
    if (qing_machine_policy_restricts_native()) return S_FALSE;
    CPINFO oem{};
    if (!GetCPInfo(CP_OEMCP, &oem) || oem.MaxCharSize > 4) return S_FALSE;
    Runtime runtime;
    runtime.init(runtime_dir);
    Source original_source(owner, false), incoming(owner);
    original_source.enumerate(original, L"original.zip", true);
    if (original_source.items.size() != 1 || original_source.items[0].directory()) fail("invalid original ZIP handle");
    for (UInt32 i = 0; i < root_count; ++i) {
      if (!roots[i].source || !roots[i].leaf || !valid_leaf(roots[i].leaf)) throw NotEligible{};
      incoming.enumerate(roots[i].source, roots[i].leaf, true);
    }
    const auto &before = original_source.items[0];
    auto bound = before.size;
    size_t count_bound = expected_count;
    std::vector<File> empty;
    for (const auto &record : incoming.items) {
      bound = next_classic_bound(bound, count_bound++, record.directory() ? 0 : record.size, record.path.size());
      if (bound == NotClassic) throw NotEligible{};
      if (!record.directory() && !record.size) {
        auto file = incoming.open(record);
        BY_HANDLE_FILE_INFORMATION observed{};
        Source::verify(file.get(), record, observed);
        empty.push_back(std::move(file));
      }
    }
    std::atomic<unsigned> verified{0};
    Ref<IInStream> input(new Input(original_source.open(before), before, original_source, verified));
    // Sources and callback state outlive the archive even if the handler
    // retains a reference while unwinding a failed update.
    std::vector<zip_append::Item> plan;
    Ref<zip_append::Update> update;
    Ref<IInArchive> archive;
    runtime.handler(L"zip", "Zip", archive.put());
    const UInt64 search = 0;
    if (archive->Open(input.get(), &search, nullptr) != S_OK) throw NotEligible{};
    try { archive_health(archive.get()); }
    catch (const Failure &) { throw NotEligible{}; }
    UInt32 count = 0;
    exact(archive->GetNumberOfItems(&count), "read original ZIP member count");
    if (count != expected_count) fail("ZIP member count changed before update");
    plan = zip_append::plan(archive.get(), count, incoming);
    *update.put() = new zip_append::Update(plan, incoming);
    Ref<IOutArchive> editor;
    exact(archive->QueryInterface(IID_IOutArchive, reinterpret_cast<void **>(editor.put())), "open ZIP update interface");
    Ref<Output> stream(new Output(output));
    exact(stream->Seek(0, FILE_BEGIN, nullptr), "set ZIP candidate position");
    frame(47);
    started = true;
    exact(editor->UpdateItems(stream.get(), static_cast<UInt32>(plan.size()), update.get()), "write updated ZIP");
    update->verify();
    incoming.live();
    if (qing_machine_policy_restricts_native()) fail("machine policy changed");
    return S_OK;
  } catch (const NotEligible &) {
    return started ? E_FAIL : S_FALSE;
  } catch (const Failure &e) {
    if (error && capacity) strncpy_s(error, capacity, e.reason, _TRUNCATE);
    return FAILED(e.hr) ? e.hr : E_FAIL;
  } catch (...) { return E_FAIL; }
}
