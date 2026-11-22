# M01 — bot-types & bot-config

**Tujuan:** Tipe domain lengkap (venue-agnostik) dan konfigurasi TOML berlapis dengan validasi ketat & aturan tighten-only.

## Baca dulu (hanya ini)
- docs/spec/03-architecture.md §3.4, §3.5 (tipe: InstrumentKey, VenueState, VenueCaps, ProtectionScheme)
- docs/spec/09-config.md (seluruhnya)
- docs/spec/07-risk.md §7.2 (tabel default), §7.8 (default Futures)

## Tugas
1. Implementasi tipe §3.4 (`Price/Qty` newtype `Decimal`, `Order` dengan `venue`, `Position` dengan `leverage/liq_price`, enum status) + tipe §3.5 (tanpa logika manajer).
2. `InstrumentRules` + fungsi pembulatan `floor_to_step`, `round_to_tick`, validasi `min_qty/min_notional` (murni, tanpa I/O).
3. `bot-config`: struct untuk **seluruh** `bot.toml` §9 (`general, safety, binance, universe, strategy.*, exit, risk, risk.futures, venues.*, ipc, storage`) dengan `serde(deny_unknown_fields)`.
4. Layering: default kode → `/etc/binance-bot/bot.toml` → `conf.d/*.toml` → env `BOT__SECTION__KEY` (figment/config).
5. Validasi rentang tiap parameter risiko (§7.2/§7.8); nilai di luar rentang = **error start** (tanpa clamp diam-diam).
6. Fungsi `is_tightening(old, new) -> Result<(), LoosenError>` untuk aturan runtime tighten-only (daftar kunci yang boleh/ tidak boleh).
7. `packaging/bot.toml.example` = salinan §9 yang lolos validasi.

## Acceptance (harus dapat dibuktikan dengan tes/perintah)
- Tes: config contoh valid; setiap parameter di luar rentang → error spesifik; kunci tak dikenal → error.
- Property test pembulatan: hasil selalu kelipatan step/tick dan tidak melebihi input (floor).
- Tes `is_tightening`: menurunkan risk% OK, menaikkan → ditolak.
- Semua tipe `Serialize/Deserialize` round-trip (Decimal sebagai string).

## Di luar cakupan (JANGAN dikerjakan di milestone ini)
- Order/koneksi sungguhan; SQLite.

## `[VERIFY]` yang harus diselesaikan & dicatat di `docs/verified-facts.md` sebelum coding
- Nama filter notional (`NOTIONAL` vs `MIN_NOTIONAL`) dan struktur `exchangeInfo.filters` (Spot) — catat.

## Penutup (wajib)
- `scripts/verify.sh` hijau; perbarui `docs/STATUS.md` (centang M01, isi catatan sesi), `CHANGELOG.md`.
- Ikuti `docs/spec/C-definition-of-done.md`. Berhenti setelah selesai; tunggu perintah pemilik untuk milestone berikutnya.
