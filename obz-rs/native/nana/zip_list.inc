// Read-only isolated ZIP directory operation. Reuses the sealed official loader
// and bounded source stream, but never exposes filesystem paths or output sinks.
namespace {
HRESULT __stdcall directory_list_trace(void *, uint32_t, uint32_t, int32_t, int32_t, uint64_t) { return S_OK; }
void emit_listing_property(IInArchive *arc, void *context, QzListingPropertyFn emit,
                           uint32_t index, uint32_t ordinal, PROPID id) {
  Variant value;
  exact(index == UINT32_MAX ? arc->GetArchiveProperty(id, &value.v)
                           : arc->GetProperty(index, id, &value.v), "archive metadata property");
  QzZipListProperty out{}; out.id = ordinal;
  const auto &v = value.v;
  switch (v.vt) {
    case VT_EMPTY: break;
    case VT_UI4: case VT_UI8: out.type = 1; out.number = number(v); break;
    case VT_BOOL: out.type = 2; out.number = boolean(v); break;
    case VT_BSTR: {
      const UINT length = SysStringLen(v.bstrVal);
      if (length > 32767 || (length && !v.bstrVal)) fail("oversized listing property");
      if (length && std::wmemchr(v.bstrVal, L'\0', length)) fail("embedded NUL in listing property");
      out.type = 3;
      out.text = v.bstrVal;
      out.text_units = length;
      break;
    }
    case VT_FILETIME:
      out.type = 4;
      out.number = (uint64_t(v.filetime.dwHighDateTime) << 32) | v.filetime.dwLowDateTime;
      out.precision = v.wReserved1; out.sub100ns = v.wReserved2; out.reserved3 = v.wReserved3;
      break;
    default: fail("unsupported archive metadata variant");
  }
  exact(emit(context, index, &out), "archive metadata consumer");
}
}

extern "C" HRESULT __stdcall qz_list_zip_at(const QzZipListRequest *request, uint32_t *item_count) {
  if (!item_count) return E_POINTER;
  *item_count = 0;
  return boundary([&]() -> HRESULT {
    static std::atomic<bool> used{false};
    if (used.exchange(true)) fail("one task per isolated ZIP worker required");
    if (!request || request->abi_version != QZ_ZIP_LIST_VERSION || request->struct_bytes != sizeof(QzZipListRequest) ||
        !request->host || request->source_size > INT64_MAX || !request->local_offset ||
        request->local_offset >= request->source_size) fail("invalid ZIP listing request");
    const auto host = *request->host;
    if (host.abi_version != QZ_ZIP_LIST_VERSION || host.struct_bytes != sizeof(QzZipListHost) ||
        !host.read_at || !host.cancelled || !host.property) fail("invalid ZIP listing host");
    // Adapter contains no output grants. VolumeStream only uses these callbacks.
    QzSelectedHost source{};
    source.context = host.context; source.read_volume_at = host.read_at;
    source.cancelled = host.cancelled; source.trace = directory_list_trace;
    auto operation = std::make_shared<OperationStatus>(source, std::wstring_view{});
    exact(operation->check(), "cancelled");
    if (effective_policy_or_unknown()) fail("native ZIP listing restricted", E_ACCESSDENIED);
    Runtime runtime; runtime.init(request->runtime);
    if (effective_policy_or_unknown()) fail("native ZIP listing policy changed", E_ACCESSDENIED);
    Ref<IInArchive> arc; runtime.handler(L"zip", "zip", arc.put());
    Ref<IInStream> input(new VolumeStream(operation, 0, request->source_size));
    exact(input->Seek(Int64(request->local_offset), STREAM_SEEK_SET, nullptr), "ZIP position");
    UInt64 search = 0;
    // No open/volume/password callback: ZIP directory encryption is unsupported
    // by this path and remains on the official console path. AES data is not read.
    exact(arc->Open(input.get(), &search, nullptr), "positioned ZIP Open");
    UInt32 count = 0; exact(arc->GetNumberOfItems(&count), "ZIP item count");
    if (count > 65536) fail("ZIP metadata item limit");
    const PROPID archive_props[] = {kpidErrorFlags, kpidWarningFlags, kpidNumErrors,
      kpidError, kpidWarning, kpidOffset, kpidPhySize, kpidTailSize};
    for (uint32_t j = 0; j < std::size(archive_props); ++j)
      emit_listing_property(arc.get(), host.context, host.property, UINT32_MAX, j, archive_props[j]);
    const PROPID item_props[] = {kpidPath, kpidIsDir, kpidSize, kpidPackSize, kpidCRC,
      kpidEncrypted, kpidMTime, kpidMethod, kpidComment, kpidAttrib, kpidSymLink,
      kpidHardLink, kpidIsAltStream, kpidPosixAttrib, kpidCTime, kpidATime};
    for (uint32_t i = 0; i < count; ++i) {
      exact(operation->check(), "ZIP metadata cancelled");
      for (uint32_t j = 0; j < std::size(item_props); ++j)
        emit_listing_property(arc.get(), host.context, host.property, i, j, item_props[j]);
    }
    exact(arc->Close(), "ZIP Close");
    exact(operation->check(), "ZIP source failed or cancelled");
    if (effective_policy_or_unknown()) fail("native ZIP listing policy changed", E_ACCESSDENIED);
    *item_count = count;
    return S_OK;
  });
}
