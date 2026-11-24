#include <QDir>
#include <QFile>
#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>
#include <QLocalServer>
#include <QLocalSocket>
#include <QProcess>
#include <QProcessEnvironment>
#include <QSignalSpy>
#include <QTemporaryDir>
#include <QtTest>

#include <unistd.h>

#include "CommandService.h"
#include "IpcClient.h"
#include "StateStore.h"

namespace {

bool waitUntil(const std::function<bool()>& pred, int timeoutMs = 5000) {
    QElapsedTimer timer;
    timer.start();
    while (timer.elapsed() < timeoutMs) {
        if (pred()) {
            return true;
        }
        QCoreApplication::processEvents(QEventLoop::AllEvents, 25);
        QTest::qWait(15);
    }
    return pred();
}

void startMockRustDaemon(QProcess& proc, const QString& sockPath) {
    const QString binPath = QStringLiteral(BOT_MOCK_UDS_BIN);
    QVERIFY2(QFile::exists(binPath), qPrintable(QStringLiteral("Missing mock_uds_daemon binary: ") + binPath));
    proc.setProgram(binPath);
    proc.setArguments({QStringLiteral("--socket"), sockPath});
    proc.setProcessChannelMode(QProcess::MergedChannels);
    proc.start();
    QVERIFY2(proc.waitForStarted(5000), "Failed to start mock_uds_daemon");
    QVERIFY2(waitUntil([&]() {
        const QByteArray out = proc.readAllStandardOutput();
        return out.contains("READY ") || QFile::exists(sockPath);
    }, 5000), "mock_uds_daemon did not bind socket in time");
}

void stopMockRustDaemon(QProcess& proc) {
    if (proc.state() == QProcess::NotRunning) {
        return;
    }
    proc.terminate();
    if (!proc.waitForFinished(2000)) {
        proc.kill();
        proc.waitForFinished(2000);
    }
}

} // namespace

class IpcUdsIntegrationTest : public QObject {
    Q_OBJECT

private slots:
    void socketDiscoveryNeverUsesImplicitTmpAndChecksPeerCred() {
        QProcessEnvironment env;
        env.insert(QStringLiteral("XDG_RUNTIME_DIR"), QStringLiteral("/run/user/1000"));
        const QString discovered = botui::IpcClient::resolveDefaultSocketPath(QString(), env);
        QVERIFY(!discovered.startsWith(QStringLiteral("/tmp/")));
        QCOMPARE(discovered, QStringLiteral("/run/user/1000/binance-bot/bot.sock"));

        QProcessEnvironment envOverride;
        envOverride.insert(QStringLiteral("BINANCE_BOT_SOCKET"), QStringLiteral("/tmp/explicit-env.sock"));
        QCOMPARE(botui::IpcClient::resolveDefaultSocketPath(QString(), envOverride),
                 QStringLiteral("/tmp/explicit-env.sock"));
        QCOMPARE(botui::IpcClient::resolveDefaultSocketPath(QStringLiteral("/tmp/explicit-cli.sock"), envOverride),
                 QStringLiteral("/tmp/explicit-cli.sock"));

        // Verify SO_PEERCRED rejects untrusted peer UID
        QTemporaryDir dir;
        QVERIFY(dir.isValid());
        const QString sockPath = dir.path() + QStringLiteral("/untrusted.sock");

        QLocalServer server;
        QVERIFY(server.listen(sockPath));

        botui::StateStore store;
        botui::IpcClient client(&store);
        // Restrict allowed UIDs to a UID that cannot match current process (e.g. uid + 99999)
        const quint32 bogusUid = static_cast<quint32>(::geteuid()) + 99999U;
        client.setAllowedPeerUids({bogusUid}, {});
        client.connectToDaemon(sockPath);

        QVERIFY(waitUntil([&]() {
            return store.lastError().contains(QStringLiteral("peer"), Qt::CaseInsensitive);
        }, 2000));
        QVERIFY(store.connectionState() != QStringLiteral("connected"));
        client.disconnectFromDaemon();
    }

    void connectsToRealRustMockUdsDaemonAndFetchesKlinesTrades() {
        QTemporaryDir dir;
        QVERIFY(dir.isValid());
        const QString sockPath = dir.path() + QStringLiteral("/rust-daemon.sock");

        QProcess daemon;
        startMockRustDaemon(daemon, sockPath);

        botui::StateStore store;
        botui::IpcClient client(&store);
        botui::CommandService commands(&store, &client);

        client.connectToDaemon(sockPath);
        QVERIFY(waitUntil([&]() {
            return store.connectionState() == QStringLiteral("connected") && store.hasSnapshot();
        }, 4000));

        QCOMPARE(store.buildMode(), QStringLiteral("paper"));
        QCOMPARE(store.engineMode(), QStringLiteral("Ready"));
        QCOMPARE(store.equity(), QStringLiteral("1012.40"));
        QCOMPARE(store.positionsModel()->rowCount(), 1);

        commands.refreshKlines(QStringLiteral("spot"), QStringLiteral("BTCUSDT"), QStringLiteral("15m"), 20);
        commands.refreshTrades(QStringLiteral("spot"));

        QVERIFY(waitUntil([&]() {
            return store.klines().size() == 20 && store.tradesModel()->rowCount() == 1;
        }, 4000));

        client.disconnectFromDaemon();
        stopMockRustDaemon(daemon);
    }

    void commandAndConfirmNonceFlowAgainstRustUds() {
        QTemporaryDir dir;
        QVERIFY(dir.isValid());
        const QString sockPath = dir.path() + QStringLiteral("/rust-confirm.sock");

        QProcess daemon;
        startMockRustDaemon(daemon, sockPath);

        botui::StateStore store;
        botui::IpcClient client(&store);
        botui::CommandService commands(&store, &client);

        client.connectToDaemon(sockPath);
        QVERIFY(waitUntil([&]() {
            return store.connectionState() == QStringLiteral("connected") && store.hasSnapshot();
        }, 4000));

        // While locked, commands are blocked client-side
        commands.pause();
        QVERIFY(store.actionFeedback().contains(QStringLiteral("locked"), Qt::CaseInsensitive));
        QCOMPARE(store.engineMode(), QStringLiteral("Ready"));

        // Unlock controls and run 1-gesture confirm flow (request_confirm -> panic_flatten)
        store.setControlsUnlocked(true);
        commands.executeConfirmedAction(QStringLiteral("panic_flatten"));

        QVERIFY(waitUntil([&]() {
            return store.engineMode() == QStringLiteral("Killed");
        }, 4000));

        // Reset kill back to Paused via nonce
        commands.executeConfirmedAction(QStringLiteral("reset_kill"));
        QVERIFY(waitUntil([&]() {
            return store.engineMode() == QStringLiteral("Paused");
        }, 4000));

        client.disconnectFromDaemon();
        stopMockRustDaemon(daemon);
    }

    void interleavedTelemetryDuringConfirmFlowAndSeqGapTriggersResync() {
        QTemporaryDir dir;
        QVERIFY(dir.isValid());
        const QString sockPath = dir.path() + QStringLiteral("/inproc.sock");

        QLocalServer server;
        QVERIFY(server.listen(sockPath));

        QLocalSocket* peer = nullptr;
        int snapshotRequests = 0;
        bool panicReceivedWithValidNonce = false;

        QObject::connect(&server, &QLocalServer::newConnection, &server, [&]() {
            peer = server.nextPendingConnection();
            auto sendLine = [peer](const QJsonObject& obj) {
                QByteArray line = QJsonDocument(obj).toJson(QJsonDocument::Compact);
                line.append('\n');
                peer->write(line);
                peer->flush();
            };

            sendLine(QJsonObject{
                {QStringLiteral("v"), 1},
                {QStringLiteral("type"), QStringLiteral("hello")},
                {QStringLiteral("seq"), 1},
                {QStringLiteral("ts_ms"), 1790000000000LL},
                {QStringLiteral("payload"), QJsonObject{
                    {QStringLiteral("protocol_version"), 1},
                    {QStringLiteral("build_mode"), QStringLiteral("paper")},
                    {QStringLiteral("capabilities"), QJsonArray{QStringLiteral("uds")}},
                    {QStringLiteral("engine_instance_id"), QStringLiteral("0192f0a8-1111-7000-8000-000000000001")},
                }},
            });

            sendLine(QJsonObject{
                {QStringLiteral("v"), 1},
                {QStringLiteral("type"), QStringLiteral("snapshot")},
                {QStringLiteral("seq"), 2},
                {QStringLiteral("ts_ms"), 1790000000010LL},
                {QStringLiteral("payload"), QJsonObject{
                    {QStringLiteral("venues"), QJsonObject{{QStringLiteral("active"), QStringLiteral("spot")}, {QStringLiteral("spot"), QStringLiteral("Active")}, {QStringLiteral("futures"), QStringLiteral("Disabled")}, {QStringLiteral("last_switch_ms"), 0}, {QStringLiteral("switches_today"), 0}}},
                    {QStringLiteral("engine"), QJsonObject{{QStringLiteral("mode"), QStringLiteral("Ready")}, {QStringLiteral("build_mode"), QStringLiteral("paper")}, {QStringLiteral("uptime_s"), 10}, {QStringLiteral("ws_latency_ms"), 5}, {QStringLiteral("clock_drift_ms"), 0}, {QStringLiteral("rate"), QJsonObject{{QStringLiteral("weight_used"), 1}, {QStringLiteral("weight_limit"), 6000}, {QStringLiteral("orders_10s"), 0}}}}},
                    {QStringLiteral("risk"), QJsonObject{{QStringLiteral("equity"), QStringLiteral("1000.00")}, {QStringLiteral("day_pnl_pct"), QStringLiteral("0.0")}, {QStringLiteral("week_pnl_pct"), QStringLiteral("0.0")}, {QStringLiteral("drawdown_pct"), QStringLiteral("0.0")}, {QStringLiteral("heat_pct"), QStringLiteral("0.0")}, {QStringLiteral("heat_max_pct"), QStringLiteral("2.0")}, {QStringLiteral("open_positions"), 0}, {QStringLiteral("max_positions"), 4}, {QStringLiteral("btc_guard"), QStringLiteral("SECURE")}, {QStringLiteral("limits"), QJsonObject{{QStringLiteral("daily_halt_pct"), QStringLiteral("-2.0")}, {QStringLiteral("kill_pct"), QStringLiteral("-10.0")}}}}},
                    {QStringLiteral("positions"), QJsonArray{}},
                    {QStringLiteral("orders"), QJsonArray{}},
                    {QStringLiteral("signals_shadow"), QJsonArray{}},
                    {QStringLiteral("alerts"), QJsonArray{}},
                }},
            });

            QObject::connect(peer, &QLocalSocket::readyRead, peer, [&, sendLine]() {
                while (peer->canReadLine()) {
                    const QByteArray raw = peer->readLine().trimmed();
                    if (raw.isEmpty()) {
                        continue;
                    }
                    const QJsonObject env = QJsonDocument::fromJson(raw).object();
                    const QString type = env.value(QStringLiteral("type")).toString();
                    if (type == QStringLiteral("request_snapshot")) {
                        ++snapshotRequests;
                        sendLine(QJsonObject{
                            {QStringLiteral("v"), 1},
                            {QStringLiteral("type"), QStringLiteral("snapshot")},
                            {QStringLiteral("seq"), 20},
                            {QStringLiteral("ts_ms"), 1790000001000LL},
                            {QStringLiteral("payload"), QJsonObject{
                                {QStringLiteral("venues"), QJsonObject{{QStringLiteral("active"), QStringLiteral("spot")}, {QStringLiteral("spot"), QStringLiteral("Active")}, {QStringLiteral("futures"), QStringLiteral("Disabled")}, {QStringLiteral("last_switch_ms"), 0}, {QStringLiteral("switches_today"), 0}}},
                                {QStringLiteral("engine"), QJsonObject{{QStringLiteral("mode"), panicReceivedWithValidNonce ? QStringLiteral("Killed") : QStringLiteral("Ready")}, {QStringLiteral("build_mode"), QStringLiteral("paper")}, {QStringLiteral("uptime_s"), 12}, {QStringLiteral("ws_latency_ms"), 5}, {QStringLiteral("clock_drift_ms"), 0}, {QStringLiteral("rate"), QJsonObject{{QStringLiteral("weight_used"), 1}, {QStringLiteral("weight_limit"), 6000}, {QStringLiteral("orders_10s"), 0}}}}},
                                {QStringLiteral("risk"), QJsonObject{{QStringLiteral("equity"), QStringLiteral("1000.00")}, {QStringLiteral("day_pnl_pct"), QStringLiteral("0.0")}, {QStringLiteral("week_pnl_pct"), QStringLiteral("0.0")}, {QStringLiteral("drawdown_pct"), QStringLiteral("0.0")}, {QStringLiteral("heat_pct"), QStringLiteral("0.0")}, {QStringLiteral("heat_max_pct"), QStringLiteral("2.0")}, {QStringLiteral("open_positions"), 0}, {QStringLiteral("max_positions"), 4}, {QStringLiteral("btc_guard"), QStringLiteral("SECURE")}, {QStringLiteral("limits"), QJsonObject{{QStringLiteral("daily_halt_pct"), QStringLiteral("-2.0")}, {QStringLiteral("kill_pct"), QStringLiteral("-10.0")}}}}},
                                {QStringLiteral("positions"), QJsonArray{}},
                                {QStringLiteral("orders"), QJsonArray{}},
                                {QStringLiteral("signals_shadow"), QJsonArray{}},
                                {QStringLiteral("alerts"), QJsonArray{}},
                            }},
                        });
                    } else if (type == QStringLiteral("command")) {
                        const QJsonObject p = env.value(QStringLiteral("payload")).toObject();
                        const QString cmdId = p.value(QStringLiteral("cmd_id")).toString();
                        const QString cmd = p.value(QStringLiteral("cmd")).toString();
                        if (cmd == QStringLiteral("request_confirm")) {
                            // Inject interleaved heartbeat BEFORE command_result!
                            sendLine(QJsonObject{
                                {QStringLiteral("v"), 1},
                                {QStringLiteral("type"), QStringLiteral("heartbeat")},
                                {QStringLiteral("seq"), 3},
                                {QStringLiteral("ts_ms"), 1790000000100LL},
                                {QStringLiteral("payload"), QJsonObject{
                                    {QStringLiteral("engine_state"), QStringLiteral("Ready")},
                                    {QStringLiteral("ws_latency_ms"), 9},
                                    {QStringLiteral("weight_used"), 10},
                                    {QStringLiteral("weight_limit"), 6000},
                                }},
                            });
                            sendLine(QJsonObject{
                                {QStringLiteral("v"), 1},
                                {QStringLiteral("type"), QStringLiteral("command_result")},
                                {QStringLiteral("seq"), 4},
                                {QStringLiteral("ts_ms"), 1790000000200LL},
                                {QStringLiteral("payload"), QJsonObject{
                                    {QStringLiteral("cmd_id"), cmdId},
                                    {QStringLiteral("ok"), true},
                                    {QStringLiteral("code"), QStringLiteral("confirm_issued")},
                                    {QStringLiteral("message"), QStringLiteral("nonce issued")},
                                    {QStringLiteral("confirm_nonce"), QStringLiteral("nonce-xyz-999")},
                                    {QStringLiteral("expires_at_ms"), QDateTime::currentMSecsSinceEpoch() + 9000},
                                }},
                            });
                        } else if (cmd == QStringLiteral("panic_flatten")) {
                            if (p.value(QStringLiteral("confirm_nonce")).toString() == QStringLiteral("nonce-xyz-999")) {
                                panicReceivedWithValidNonce = true;
                            }
                            sendLine(QJsonObject{
                                {QStringLiteral("v"), 1},
                                {QStringLiteral("type"), QStringLiteral("command_result")},
                                {QStringLiteral("seq"), 5},
                                {QStringLiteral("ts_ms"), 1790000000300LL},
                                {QStringLiteral("payload"), QJsonObject{
                                    {QStringLiteral("cmd_id"), cmdId},
                                    {QStringLiteral("ok"), panicReceivedWithValidNonce},
                                    {QStringLiteral("code"), QStringLiteral("ok")},
                                    {QStringLiteral("message"), QStringLiteral("flattened")},
                                }},
                            });
                        }
                    }
                }
            });
        });

        botui::StateStore store;
        botui::IpcClient client(&store);
        botui::CommandService commands(&store, &client);
        client.connectToDaemon(sockPath);

        QVERIFY(waitUntil([&]() { return store.hasSnapshot(); }, 3000));
        store.setControlsUnlocked(true);
        commands.executeConfirmedAction(QStringLiteral("panic_flatten"));

        QVERIFY(waitUntil([&]() { return panicReceivedWithValidNonce; }, 3000));

        // Now inject an out-of-order seq jump (jump to seq 99) and verify client sends request_snapshot
        const int beforeGapRequests = snapshotRequests;
        QVERIFY(peer != nullptr);
        QByteArray gapFrame = QJsonDocument(QJsonObject{
            {QStringLiteral("v"), 1},
            {QStringLiteral("type"), QStringLiteral("heartbeat")},
            {QStringLiteral("seq"), 99},
            {QStringLiteral("ts_ms"), 1790000000900LL},
            {QStringLiteral("payload"), QJsonObject{
                {QStringLiteral("engine_state"), QStringLiteral("Killed")},
                {QStringLiteral("ws_latency_ms"), 9},
                {QStringLiteral("weight_used"), 10},
                {QStringLiteral("weight_limit"), 6000},
            }},
        }).toJson(QJsonDocument::Compact);
        gapFrame.append('\n');
        peer->write(gapFrame);
        peer->flush();

        QVERIFY(waitUntil([&]() { return snapshotRequests > beforeGapRequests; }, 3000));
        client.disconnectFromDaemon();
    }

    void autoReconnectsWhenRustDaemonRestarts() {
        QTemporaryDir dir;
        QVERIFY(dir.isValid());
        const QString sockPath = dir.path() + QStringLiteral("/restart.sock");

        QProcess daemon1;
        startMockRustDaemon(daemon1, sockPath);

        botui::StateStore store;
        botui::IpcClient client(&store);
        client.connectToDaemon(sockPath);

        QVERIFY(waitUntil([&]() {
            return store.connectionState() == QStringLiteral("connected") && store.hasSnapshot();
        }, 4000));
        const QString firstInstanceId = store.engineInstanceId();
        QVERIFY(!firstInstanceId.isEmpty());

        // Kill daemon1 and verify client enters reconnecting state
        stopMockRustDaemon(daemon1);
        QVERIFY(waitUntil([&]() {
            return store.connectionState() == QStringLiteral("reconnecting");
        }, 3000));

        // Start a brand-new daemon2 on the same socket path; client must auto-reconnect without calling connectToDaemon
        QProcess daemon2;
        startMockRustDaemon(daemon2, sockPath);
        QVERIFY(waitUntil([&]() {
            return store.connectionState() == QStringLiteral("connected") &&
                   !store.engineInstanceId().isEmpty() &&
                   store.engineInstanceId() != firstInstanceId;
        }, 6000));

        client.disconnectFromDaemon();
        stopMockRustDaemon(daemon2);
    }
};

QTEST_MAIN(IpcUdsIntegrationTest)
#include "tst_ipc_uds_integration.moc"
