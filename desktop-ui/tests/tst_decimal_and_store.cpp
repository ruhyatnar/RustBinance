#include <QDir>
#include <QFile>
#include <QJsonDocument>
#include <QJsonObject>
#include <QSignalSpy>
#include <QtTest>

#include "DecimalFormat.h"
#include "Models.h"
#include "StateStore.h"

namespace {

QJsonObject loadFixtureObject(const QString& fileName) {
    const QString path = QStringLiteral(BOT_REPO_ROOT) + QStringLiteral("/protocol/fixtures/v1/") + fileName;
    QFile file(path);
    if (!file.open(QIODevice::ReadOnly)) {
        return {};
    }
    const QJsonDocument doc = QJsonDocument::fromJson(file.readAll());
    return doc.object();
}

} // namespace

class DecimalAndStoreTest : public QObject {
    Q_OBJECT

private slots:
    void formatsDecimalStringsWithoutFloatingPointPrecisionLoss() {
        botui::DecimalFormat fmt;
        QCOMPARE(fmt.formatDecimal(QStringLiteral("-1012345.67890000")),
                 QStringLiteral("-1,012,345.67890000"));
        QCOMPARE(fmt.formatDecimal(QStringLiteral("999999999999999999.12345678")),
                 QStringLiteral("999,999,999,999,999,999.12345678"));
        QCOMPARE(fmt.formatDecimal(QStringLiteral("1012.40"), 2),
                 QStringLiteral("1,012.40"));
        QCOMPARE(fmt.formatDecimal(QStringLiteral("3120.4567"), 2),
                 QStringLiteral("3,120.45"));
        QCOMPARE(fmt.formatDecimal(QStringLiteral("")), QStringLiteral("—"));

        QCOMPARE(fmt.decimalSign(QStringLiteral("-0.00")), 0);
        QCOMPARE(fmt.decimalSign(QStringLiteral("0")), 0);
        QCOMPARE(fmt.decimalSign(QStringLiteral("1.9")), 1);
        QCOMPARE(fmt.decimalSign(QStringLiteral("-1.1")), -1);

        QCOMPARE(fmt.directionGlyph(QStringLiteral("1.9")), QStringLiteral("▲"));
        QCOMPARE(fmt.directionGlyph(QStringLiteral("-1.1")), QStringLiteral("▼"));
        QCOMPARE(fmt.directionGlyph(QStringLiteral("0.00")), QStringLiteral("•"));
    }

    void appliesProtocolFixturesDetectsSeqGapsAndMergesDeltas() {
        botui::StateStore store;
        QCOMPARE(store.canMutate(), false);

        store.setConnectionState(QStringLiteral("connected"));
        QCOMPARE(store.canMutate(), false);
        store.setControlsUnlocked(true);
        QCOMPARE(store.canMutate(), true);

        const QJsonObject hello = loadFixtureObject(QStringLiteral("hello.json"));
        QCOMPARE(store.applyEnvelope(hello), botui::EnvelopeOutcome::Ok);
        QCOMPARE(store.buildMode(), QStringLiteral("paper"));
        QCOMPARE(store.protocolVersion(), 1);

        const QJsonObject snap = loadFixtureObject(QStringLiteral("snapshot.json"));
        QCOMPARE(store.applyEnvelope(snap), botui::EnvelopeOutcome::Ok);
        QCOMPARE(store.engineMode(), QStringLiteral("Ready"));
        QCOMPARE(store.activeVenue(), QStringLiteral("spot"));
        QCOMPARE(store.equity(), QStringLiteral("1012.40"));
        QCOMPARE(store.positionsModel()->rowCount(), 1);

        const QModelIndex posIdx = store.positionsModel()->index(0, 0);
        QCOMPARE(store.positionsModel()->data(posIdx, botui::PositionsModel::VenueRole).toString(),
                 QStringLiteral("spot"));
        QCOMPARE(store.positionsModel()->data(posIdx, botui::PositionsModel::SymbolRole).toString(),
                 QStringLiteral("ETHUSDT"));
        QCOMPARE(store.positionsModel()->data(posIdx, botui::PositionsModel::ProfitRRole).toString(),
                 QStringLiteral("1.9"));

        const QJsonObject delta = loadFixtureObject(QStringLiteral("delta.json"));
        QCOMPARE(store.applyEnvelope(delta), botui::EnvelopeOutcome::Ok);
        QCOMPARE(store.equity(), QStringLiteral("1015.00"));
        QCOMPARE(store.positionsModel()->rowCount(), 1); // preserved when delta.positions is null

        // Inject a seq gap: lastSeq is 3, jump to 5
        QSignalSpy gapSpy(&store, &botui::StateStore::seqGapDetected);
        QJsonObject gapped = loadFixtureObject(QStringLiteral("heartbeat.json"));
        gapped.insert(QStringLiteral("seq"), 5);
        QCOMPARE(store.applyEnvelope(gapped), botui::EnvelopeOutcome::SeqGap);
        QCOMPARE(gapSpy.count(), 1);

        // Fresh snapshot resets baseline even after gap
        QJsonObject resyncSnap = snap;
        resyncSnap.insert(QStringLiteral("seq"), 10);
        QCOMPARE(store.applyEnvelope(resyncSnap), botui::EnvelopeOutcome::Ok);
        QCOMPARE(store.lastSeq(), static_cast<qulonglong>(10));

        // Apply market, event, klines, trades with contiguous seq 11..14
        QJsonObject market = loadFixtureObject(QStringLiteral("market.json"));
        market.insert(QStringLiteral("seq"), 11);
        QCOMPARE(store.applyEnvelope(market), botui::EnvelopeOutcome::Ok);
        QVERIFY(store.marketTicksModel()->rowCount() >= 1);

        QJsonObject ev = loadFixtureObject(QStringLiteral("event.json"));
        ev.insert(QStringLiteral("seq"), 12);
        QCOMPARE(store.applyEnvelope(ev), botui::EnvelopeOutcome::Ok);
        QCOMPARE(store.eventsModel()->rowCount(), 1);

        QJsonObject klines = loadFixtureObject(QStringLiteral("klines.json"));
        klines.insert(QStringLiteral("seq"), 13);
        QCOMPARE(store.applyEnvelope(klines), botui::EnvelopeOutcome::Ok);
        QCOMPARE(store.klines().size(), 2);

        QJsonObject trades = loadFixtureObject(QStringLiteral("trades.json"));
        trades.insert(QStringLiteral("seq"), 14);
        QCOMPARE(store.applyEnvelope(trades), botui::EnvelopeOutcome::Ok);
        QCOMPARE(store.tradesModel()->rowCount(), 1);
    }

    void deduplicatesAlertNotificationsAcrossSnapshots() {
        botui::StateStore store;
        QSignalSpy notifSpy(&store, &botui::StateStore::criticalNotificationRequested);

        QJsonObject snap1 = loadFixtureObject(QStringLiteral("snapshot.json"));
        snap1.insert(QStringLiteral("seq"), 1);
        QCOMPARE(store.applyEnvelope(snap1), botui::EnvelopeOutcome::Ok);
        QCOMPARE(notifSpy.count(), 1); // unacked alert "a1" fires once

        // Re-applying snapshot (e.g. after reconnect/resync) with the exact same alert id "a1" must NOT re-fire
        QJsonObject snap2 = snap1;
        snap2.insert(QStringLiteral("seq"), 2);
        QCOMPARE(store.applyEnvelope(snap2), botui::EnvelopeOutcome::Ok);
        QCOMPARE(notifSpy.count(), 1);
    }
};

QTEST_MAIN(DecimalAndStoreTest)
#include "tst_decimal_and_store.moc"
