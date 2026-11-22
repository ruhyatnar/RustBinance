# ADR-0001 — Kode inti venue-agnostik sejak awal

- Status: Diterima
- Konteks: Spot dan Futures harus dapat berganti mode saat runtime; simbol sama (`BTCUSDT`) ada di kedua venue. Urutan build Spot dulu (M02–M14), Futures menyusul (M15–M16).
- Keputusan: semua tipe/DB/IPC memakai `Venue` dan kunci `(Venue, Symbol)` sejak M00–M01; `VenueAdapter` adalah satu-satunya batas ke exchange; `clientOrderId` memuat huruf venue.
- Konsekuensi: M15–M16 hanya menambah adapter dan manajer — bukan refaktor besar. Kode yang mengasumsikan Spot-only dianggap bug.
