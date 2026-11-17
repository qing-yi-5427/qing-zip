#include "zip_list_packet.h"
namespace {
HRESULT __stdcall list_property(void *context, uint32_t index, const QzZipListProperty *p) {
  auto &w = *static_cast<Worker *>(context);
  return guarded(w, [&]() -> HRESULT {
    auto msg = w.message();
    put(msg, index); put(msg, p->id); put(msg, p->type); put(msg, p->number);
    put(msg, p->precision); put(msg, p->sub100ns); put(msg, p->reserved3);
    put(msg, p->text_units);
    if (p->text_units) append(msg, p->text, size_t(p->text_units) * 2);
    zip_list_send_property(msg, [&](uint8_t *packet, size_t length) {
      std::lock_guard<std::mutex> lock(w.channel.write_lock);
      transfer(w.channel.output, packet, length, true);
    }, [&] { w.channel.send(9, msg); });
    return S_OK;
  });
}
}
extern "C" int qing_zip_list_worker_main(const wchar_t *runtime) noexcept {
  try {
    Worker w; w.start(runtime, 8);
    if (w.sources.size() != 1 || !w.password.empty()) return 3;
    auto position = w.channel.read(10); w.match_nonce(position);
    if (position.get<uint32_t>() != QZ_ZIP_LIST_VERSION) return 3;
    auto offset = position.get<uint64_t>(); position.end();
    QzZipListHost host{QZ_ZIP_LIST_VERSION, sizeof(QzZipListHost), &w, read_at, cancelled, list_property};
    QzZipListRequest request{QZ_ZIP_LIST_VERSION, sizeof(QzZipListRequest), runtime,
      w.volumes[0].fixed_size, offset, &host};
    uint32_t count = 0;
    const auto hr = qz_list_zip_at(&request, &count);
    auto msg = w.message(); put(msg, uint32_t(hr)); put(msg, count);
    w.channel.send(11, msg);
    return hr == S_OK && !w.failed ? 0 : 2;
  } catch (...) { return 3; }
}
