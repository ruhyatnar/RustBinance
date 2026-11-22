## 1. Hasil Audit v2 → Perbaikan v3 (Ringkasan)

Hasil review blueprint v2 + riset dokumentasi Binance & literatur (Bagian 15). Item bertanda **KRITIS** dapat menyebabkan kehilangan dana atau bot tidak jalan.

| # | Temuan di v2 | Dampak | Perbaikan v3 |
|---|---|---|---|
| 1 | **KRITIS** — ListenKey keep-alive 30 menit untuk Spot. Mekanisme listenKey Spot **sudah dihapus 2026-02-20**; penggantinya adalah `session.logon` (Ed25519) + `userDataStream.subscribe` di koneksi WS API | Bot tidak menerima event fill/akun | Bagian 4.2: user data via WS API; listenKey hanya untuk Futures bila masih berlaku `[VERIFY]` |
| 2 | **KRITIS** — Order kondisional Futures (STOP_MARKET, TAKE_PROFIT_MARKET, TRAILING_STOP_MARKET, dst.) sudah pindah ke **Algo Service** (error `-4120` bila lewat `/fapi/v1/order`); event baru `ALGO_UPDATE` | SL/TP tidak terpasang | Bagian 4.5 (Futures) memakai `algoOrder` |
| 3 | **KRITIS** — `place_limit_maker_order` membuka koneksi WS baru **per order** dan membaca pesan pertama apa pun | Latensi tinggi, salah-korelasi respons, kena limit koneksi (300/5 menit/IP) | Satu koneksi persisten + peta korelasi `id → oneshot` (4.2) |
| 4 | Koneksi WS API hanya valid **24 jam** — tidak ditangani | Disconnect mendadak | Rotasi proaktif ±23 jam, make-before-break (4.2) |
| 5 | **KRITIS** — `f64` untuk harga/qty/saldo | Galat pembulatan → order ditolak filter / selisih saldo | `Decimal` + newtype + pembulatan filter (3.4) |
| 6 | `sled` sebagai persistence (tidak mencapai rilis stabil, tidak aktif dirawat) | Risiko korupsi/abandon | SQLite WAL (journal event + snapshot) (Bagian 8) |
| 7 | **KRITIS (keamanan)** — UDS di `/tmp`, endpoint `POST /api/panic-flatten` **tanpa autentikasi**, web gateway bind tanpa batas | Siapa pun di host/jaringan bisa menutup posisi / mengendalikan bot | Socket di `/run/binance-bot/`, mode 0660 grup; web bind `127.0.0.1`, token + cek Origin; perintah destruktif butuh konfirmasi (Bagian 10) |
| 8 | Default `UsdsFutures`, short, risiko 1% × 5 posisi searah | Aset kripto berkorelasi tinggi → risiko efektif ≈ 5% per satu gerakan pasar; ada risiko likuidasi | Risiko default 0.5% (Futures 0.35%); batas **portfolio heat** lintas-venue; **Spot dan Futures sama-sama didukung dan dapat berganti mode saat runtime** (3.5); Futures: leverage ≤3×, isolated, buffer likuidasi (7.8) |
| 9 | Strategi 5m tanpa pagar biaya (fee) | Biaya round-trip Spot (≈0.1% per sisi default `[VERIFY]`) bisa lebih besar dari edge | **Cost gate** (target ≥ 3× biaya round-trip), TF default 1h/4h (Bagian 6) |
| 10 | Kelly 0.25× dari win-rate historis **per simbol** | Sampel kecil → estimasi liar → sizing berbahaya | Fixed-fractional default; Kelly hanya sebagai *pengurang* dan butuh n ≥ 100 (7.3) |
| 11 | BTC Crash Guard tanpa kondisi rilis | Bot terkunci selamanya / atau flapping | Histeresis + cooldown (7.5) |
| 12 | Urutan cek lifecycle: SL dicek paling akhir; TP/SL dihitung dari tick lokal | Keterlambatan keluar | SL dicek **pertama**; proteksi selalu **di sisi exchange** (OCO/algo) + watcher software sebagai cadangan (4.4, 5.5) |
| 13 | Order book `@depth@100ms` tanpa prosedur sinkronisasi snapshot/diff | Book korup → OBI salah | Prosedur resmi sinkronisasi + resync otomatis (4.3) |
| 14 | Kode error keliru/tidak terverifikasi (`-4048` untuk reduce-only; `-5022` dipakai untuk Spot) | Handler error salah | Tabel error terverifikasi (Lampiran A); `-5022` hanya Futures GTX; Spot LIMIT_MAKER ditolak sebagai `-2010` `[VERIFY]` |
| 15 | Konfigurasi hanya `.env` flat, semua parameter bisa diubah | Salah ubah risiko saat runtime | `bot.toml` berlapis + validasi; runtime hanya boleh **memperketat** risiko (Bagian 9) |
| 16 | Tidak ada tahap backtest/paper/gerbang rilis | Langsung live tanpa bukti | Pipeline validasi & kriteria lulus numerik (Bagian 13) |
| 17 | Installer berorientasi apt/root, build sebagai root-escalation, blok kode terpecah, bagian 11.1 duplikat, `bot-keygen` tidak ada di tree, rujukan `strategi.md` tidak ada | Gagal dijalankan oleh agen | Installer ulang **CachyOS/Arch-first** (pacman), unit systemd ter-harden, referensi dibersihkan (Bagian 12) |
| 18 | Izin API: Spot+Margin+Futures sekaligus | Permukaan serangan besar | Least privilege: **API key & keypair terpisah per venue**, izin minimum; **withdrawal & transfer antar-wallet MUST dimatikan**; whitelist IP |
| 19 | `bot-keygen` menulis file lalu `chmod` (race izin) | Private key sempat terbaca | Buat file dengan mode `0600` sejak awal (Bagian 12.3) |
| 20 | Throttler UI berbasis tick buta | Frame lama menumpuk / tidak ada conflation | Conflation per-key, `MissedTickBehavior::Skip` (10.4) |
| 21 | Venue dikunci satu nilai di config (Spot **atau** Futures) | Tidak bisa berpindah tanpa restart; risiko state campur antar-venue (simbol sama `BTCUSDT` ada di keduanya) | `VenueManager`: peralihan runtime dengan state machine, isolasi kredensial/state per venue, kunci `(Venue, Symbol)`, risiko lintas-venue (3.5, 7.8) |
