// Use the same official Rar/Rar5 handlers as NanaZip's UI agent. In the
// pinned upstream AgentProxy.cpp and Agent.cpp the UI gets properties directly
// from IInArchive; ArchiveOpenCallback.cpp supplies the requested volume streams.
extern "C" HRESULT __stdcall qz_list_rar(
    const wchar_t *directory, const QzSelectedVolume *volumes, uint32_t volume_count,
    bool rar5, const QzRarListHost *host, const wchar_t *password, uint32_t password_units,
    uint32_t *item_count, uint32_t *phase) {
  *item_count = 0;
  *phase = 0;
  return boundary([&]() -> HRESULT {
    QzSelectedHost source{};
    source.context = host->context;
    source.read_volume_at = host->read_at;
    source.cancelled = host->cancelled;
    source.trace = directory_list_trace;
    auto state = std::make_shared<OperationStatus>(source,
        password_units ? std::wstring_view(password, password_units) : std::wstring_view{});
    exact(state->check(), "RAR listing cancelled");
    if (effective_policy_or_unknown()) fail("native RAR listing restricted", E_ACCESSDENIED);
    *phase = 1;
    Runtime runtime;
    runtime.init(directory);
    if (effective_policy_or_unknown()) fail("native RAR listing policy changed", E_ACCESSDENIED);
    *phase = 2;
    VolumeMap map(volumes, volume_count, state);
    Ref<IArchiveOpenCallback> callback(new OpenCallback(map, true, true, true));
    Ref<IInStream> stream(new VolumeStream(state, 0, map.sizes[0]));
    Ref<IInArchive> arc;
    *phase = 3;
    runtime.handler(rar5 ? L"Rar5" : L"Rar", rar5 ? "Rar5" : "Rar", arc.put());
    const UInt64 search = 0;
    *phase = 4;
    exact(arc->Open(stream.get(), &search, callback.get()), "RAR open");
    { Variant v; exact(arc->GetArchiveProperty(kpidError, &v.v), "RAR diagnostic");
      if (text(v.v, true).starts_with(L"Missing volume : "))
        return HRESULT_FROM_WIN32(ERROR_FILE_NOT_FOUND);
    }
    *phase = 5;
    archive_health(arc.get());
    { Variant v; exact(arc->GetArchiveProperty(kpidOffset, &v.v), "RAR offset");
      if (number(v.v, true)) return S_FALSE;
    }
    *phase = 6;
    { Variant n; exact(arc->GetArchiveProperty(kpidNumVolumes, &n.v), "RAR volume count");
      const auto used = number(n.v);
      if (used != volume_count) return S_FALSE;
      // Split archives can pad volumes to the requested size. The first
      // volume's extent and the combined extent come from the decoder.
      // Single-file wrappers/tails retain the format-discovery console route.
      Variant first;
      exact(arc->GetArchiveProperty(kpidPhySize, &first.v), "RAR first volume extent");
      const auto first_bytes = number(first.v);
      if (!first_bytes || first_bytes > map.sizes[0]) return S_FALSE;
      if (used == 1) {
        if (first_bytes != map.total) return S_FALSE;
      } else {
        Variant size;
        exact(arc->GetArchiveProperty(kpidTotalPhySize, &size.v), "RAR total extent");
        const auto physical = number(size.v);
        if (physical < first_bytes || physical > map.total) return S_FALSE;
      }
    }
    *phase = 7;
    UInt32 count = 0;
    exact(arc->GetNumberOfItems(&count), "RAR item count");
    if (count > QZ_RAR_LIST_MAX_ITEMS) return HRESULT_FROM_WIN32(ERROR_BUFFER_OVERFLOW);
    exact(host->begin_items(host->context, count), "RAR item total");
    const PROPID props[] = {kpidPath, kpidIsDir, kpidSize, kpidPackSize, kpidCRC,
      kpidEncrypted, kpidMTime, kpidMethod, kpidComment, kpidAttrib, kpidSymLink,
      kpidHardLink, kpidIsAltStream, kpidPosixAttrib, kpidCTime, kpidATime};
    *phase = 8;
    for (UInt32 index = 0; index < count; ++index) {
      exact(state->check(), "RAR listing cancelled");
      for (PROPID prop : {kpidSplitBefore, kpidSplitAfter}) {
        Variant flag; exact(arc->GetProperty(index, prop, &flag.v), "RAR split status");
        if (flag.v.vt != VT_EMPTY && flag.v.vt != VT_BOOL) return S_FALSE;
        if (flag.v.vt == VT_BOOL && flag.v.boolVal != VARIANT_FALSE) return HRESULT_FROM_WIN32(ERROR_FILE_NOT_FOUND);
      }
      for (uint32_t ordinal = 0; ordinal < std::size(props); ++ordinal)
        emit_listing_property(arc.get(), host->context, host->property, index, ordinal, props[ordinal]);
    }
    *phase = 9;
    exact(arc->Close(), "RAR close");
    exact(state->check(), "RAR listing cancelled");
    if (effective_policy_or_unknown()) fail("native RAR listing policy changed", E_ACCESSDENIED);
    *item_count = count;
    return S_OK;
  });
}
