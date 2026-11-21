namespace {
struct RarList {
  Worker &worker;
  Bytes batch;
  uint64_t sent = 0;
  uint32_t rows = 0;
  std::atomic<ULONGLONG> last_parent_check{0};

  explicit RarList(Worker &w) : worker(w), batch(w.message()) {
    put(batch, rows);
    batch.reserve(256 * 1024);
  }

  HRESULT flush() {
    if (!rows) return S_OK;
    if (batch.size() > QZ_RAR_LIST_MAX_BYTES - sent)
      return HRESULT_FROM_WIN32(ERROR_BUFFER_OVERFLOW);
    memcpy(batch.data() + 16, &rows, sizeof(rows));
    sent += batch.size();
    worker.channel.send(9, batch);
    batch.resize(16 + sizeof(rows));
    rows = 0;
    return S_OK;
  }
};

HRESULT __stdcall rar_list_property(void *context, uint32_t, const QzZipListProperty *p) {
  auto &list = *static_cast<RarList *>(context);
  return guarded(list.worker, [&]() -> HRESULT {
    // Batch only complete rows. A type byte precedes each variable-width value.
    put(list.batch, uint8_t(p->type));
    switch (p->type) {
      case 0: break;
      case 1: put(list.batch, p->number); break;
      case 2: put(list.batch, uint8_t(p->number)); break;
      case 3:
        put(list.batch, p->text_units);
        if (p->text_units) append(list.batch, p->text, size_t(p->text_units) * 2);
        break;
      case 4:
        put(list.batch, p->number);
        put(list.batch, p->precision);
        put(list.batch, p->sub100ns);
        put(list.batch, p->reserved3);
        break;
      default: return E_FAIL;
    }
    if (p->id == 15) {
      ++list.rows;
      if (list.batch.size() >= 256 * 1024) return list.flush();
    }
    return S_OK;
  });
}
HRESULT __stdcall rar_list_begin(void *context, uint32_t count) {
  auto &w = static_cast<RarList *>(context)->worker;
  return guarded(w, [&]() -> HRESULT {
    auto message = w.message();
    put(message, count);
    w.channel.send(12, message);
    return S_OK;
  });
}
HRESULT __stdcall rar_list_read(void *context, uint32_t index, uint64_t offset,
                                void *buffer, uint32_t size, uint32_t *got) {
  return read_at(&static_cast<RarList *>(context)->worker, index, offset, buffer, size, got);
}
int32_t __stdcall rar_list_cancelled(void *context) {
  auto &list = *static_cast<RarList *>(context);
  if (list.worker.failed) return 1;
  const ULONGLONG now = GetTickCount64();
  auto previous = list.last_parent_check.load(std::memory_order_relaxed);
  if (now - previous >= 25 &&
      list.last_parent_check.compare_exchange_strong(previous, now, std::memory_order_relaxed) &&
      WaitForSingleObject(list.worker.parent, 0) != WAIT_TIMEOUT) {
    list.worker.failed = true;
    return 1;
  }
  // The parent also polls the job's cancellation token while reading frames.
  // Limit kernel waits during the RAR header scan and per-property callbacks.
  return 0;
}
}
extern "C" int qing_rar_list_worker_main(const wchar_t *runtime) noexcept {
  try {
    Worker w;
    w.start(runtime, 12);
    auto request = w.channel.read(10);
    w.match_nonce(request);
    if (request.get<uint32_t>() != QZ_RAR_LIST_VERSION) return 3;
    auto format = request.get<uint32_t>();
    request.end();
    if (format != 4 && format != 5) return 3;
    RarList list(w);
    QzRarListHost host{&list, rar_list_read,
                       rar_list_cancelled, rar_list_property, rar_list_begin};
    uint32_t count = 0, phase = 0;
    auto hr = qz_list_rar(runtime, w.volumes.data(), uint32_t(w.volumes.size()),
                                format == 5, &host, w.password.data(), uint32_t(w.password.size()), &count, &phase);
    if (hr == S_OK) hr = list.flush();
    auto receipt = w.message();
    put(receipt, uint32_t(hr));
    put(receipt, count);
    put(receipt, phase);
    w.channel.send(11, receipt);
    return hr == S_OK && !w.failed ? 0 : 2;
  } catch (...) { return 3; }
}
