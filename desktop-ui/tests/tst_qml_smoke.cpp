#include <QApplication>
#include <QDirIterator>
#include <QFile>
#include <QJsonDocument>
#include <QJsonObject>
#include <QQmlApplicationEngine>
#include <QQmlComponent>
#include <QQmlContext>
#include <QQuickItem>
#include <QQuickWindow>
#include <QSignalSpy>
#include <QtQml/qqmlextensionplugin.h>
#include <QtTest>

#include "CommandService.h"
#include "DecimalFormat.h"
#include "IpcClient.h"
#include "Notifier.h"
#include "StateStore.h"

Q_IMPORT_QML_PLUGIN(BinanceBotPlugin)

namespace {

QStringList g_capturedWarnings;

void warningCollector(QtMsgType type, const QMessageLogContext&, const QString& msg) {
    if (type == QtWarningMsg || type == QtCriticalMsg || type == QtFatalMsg) {
        // Ignore benign XDG_RUNTIME_DIR notice emitted by QStandardPaths in container/CI shells
        if (msg.contains(QStringLiteral("XDG_RUNTIME_DIR not set"))) {
            return;
        }
        g_capturedWarnings.push_back(msg);
    }
}

QJsonObject loadFixture(const QString& name) {
    QFile f(QStringLiteral(BOT_REPO_ROOT) + QStringLiteral("/protocol/fixtures/v1/") + name);
    if (!f.open(QIODevice::ReadOnly)) {
        return {};
    }
    return QJsonDocument::fromJson(f.readAll()).object();
}

} // namespace

class QmlSmokeTest : public QObject {
    Q_OBJECT

private slots:
    void loadsMainWindowAndRendersAllNinePagesWithZeroWarnings() {
        g_capturedWarnings.clear();
        const QtMessageHandler prevHandler = qInstallMessageHandler(warningCollector);

        {
            botui::StateStore store;
            botui::IpcClient client(&store);
            botui::CommandService commands(&store, &client);
            botui::DecimalFormat fmt;
            botui::Notifier notifier(&store, &commands);

            QQmlApplicationEngine engine;
            engine.rootContext()->setContextProperty(QStringLiteral("stateStore"), &store);
            engine.rootContext()->setContextProperty(QStringLiteral("ipcClient"), &client);
            engine.rootContext()->setContextProperty(QStringLiteral("commandService"), &commands);
            engine.rootContext()->setContextProperty(QStringLiteral("decimalFormat"), &fmt);
            engine.rootContext()->setContextProperty(QStringLiteral("notifier"), &notifier);

            engine.load(QUrl(QStringLiteral("qrc:/qt/qml/BinanceBot/qml/Main.qml")));
            QVERIFY2(!engine.rootObjects().isEmpty(),
                     qPrintable(QStringLiteral("Main.qml failed to load; warnings: ") +
                                g_capturedWarnings.join(QStringLiteral(" | "))));

            auto* win = qobject_cast<QQuickWindow*>(engine.rootObjects().first());
            QVERIFY(win != nullptr);

            // Feed live protocol fixtures into StateStore
            store.setConnectionState(QStringLiteral("connected"));
            store.setControlsUnlocked(true);
            QJsonObject hello = loadFixture(QStringLiteral("hello.json"));
            hello.insert(QStringLiteral("seq"), 1);
            store.applyEnvelope(hello);

            QJsonObject snap = loadFixture(QStringLiteral("snapshot.json"));
            snap.insert(QStringLiteral("seq"), 2);
            store.applyEnvelope(snap);

            QJsonObject market = loadFixture(QStringLiteral("market.json"));
            market.insert(QStringLiteral("seq"), 3);
            store.applyEnvelope(market);

            QJsonObject ev = loadFixture(QStringLiteral("event.json"));
            ev.insert(QStringLiteral("seq"), 4);
            store.applyEnvelope(ev);

            store.setChartSymbol(QStringLiteral("BTCUSDT"));
            QJsonObject klines = loadFixture(QStringLiteral("klines.json"));
            klines.insert(QStringLiteral("seq"), 5);
            store.applyEnvelope(klines);

            QJsonObject trades = loadFixture(QStringLiteral("trades.json"));
            trades.insert(QStringLiteral("seq"), 6);
            store.applyEnvelope(trades);

            // Visit all 9 screens (0..8) and pump events so every binding/canvas renders
            for (int pageIdx = 0; pageIdx < 9; ++pageIdx) {
                QVERIFY(win->setProperty("currentPageIndex", pageIdx));
                QCoreApplication::processEvents(QEventLoop::AllEvents, 50);
                QTest::qWait(20);
                QCOMPARE(win->property("currentPageIndex").toInt(), pageIdx);
            }
        }

        qInstallMessageHandler(prevHandler);
        QVERIFY2(g_capturedWarnings.isEmpty(),
                 qPrintable(QStringLiteral("Unexpected Qt/QML warnings: ") +
                            g_capturedWarnings.join(QStringLiteral("\n"))));
    }

    void holdToConfirmRejectsShortPressAndKeyboardAndFiresAfterFullHold() {
        QQmlEngine engine;
        QQmlComponent comp(&engine,
                           QUrl(QStringLiteral("qrc:/qt/qml/BinanceBot/qml/components/HoldToConfirmButton.qml")));
        QVERIFY2(comp.isReady(), qPrintable(comp.errorString()));

        QQuickWindow hostWindow;
        hostWindow.resize(240, 80);
        hostWindow.show();

        std::unique_ptr<QObject> obj(comp.create());
        auto* item = qobject_cast<QQuickItem*>(obj.get());
        QVERIFY(item != nullptr);
        item->setParentItem(hostWindow.contentItem());
        item->setProperty("holdMs", 180); // shorten timer slightly for deterministic unit test speed while testing exact logic
        item->setFocus(true);

        QSignalSpy confirmedSpy(item, SIGNAL(confirmed()));
        QVERIFY(confirmedSpy.isValid());

        // 1. Space / Enter key press MUST NOT trigger confirmed()
        QTest::keyClick(&hostWindow, Qt::Key_Space);
        QTest::keyClick(&hostWindow, Qt::Key_Return);
        QTest::qWait(220);
        QCOMPARE(confirmedSpy.count(), 0);

        // 2. Short pointer press (60ms < 180ms) MUST cancel and NOT emit confirmed()
        const QPoint center(static_cast<int>(item->width() / 2), static_cast<int>(item->height() / 2));
        QTest::mousePress(&hostWindow, Qt::LeftButton, Qt::NoModifier, center);
        QTest::qWait(60);
        QTest::mouseRelease(&hostWindow, Qt::LeftButton, Qt::NoModifier, center);
        QTest::qWait(180);
        QCOMPARE(confirmedSpy.count(), 0);

        // 3. Continuous hold >= holdMs MUST emit confirmed() exactly once
        QTest::mousePress(&hostWindow, Qt::LeftButton, Qt::NoModifier, center);
        QTest::qWait(240);
        QTest::mouseRelease(&hostWindow, Qt::LeftButton, Qt::NoModifier, center);
        QCOMPARE(confirmedSpy.count(), 1);
    }

    void qmlSourcesContainZeroHardcodedDomainNumbers() {
        const QString qmlRoot = QStringLiteral(BOT_QML_SOURCE_DIR);
        QDirIterator it(qmlRoot, QStringList{QStringLiteral("*.qml")}, QDir::Files,
                        QDirIterator::Subdirectories);

        const QStringList forbiddenLiterals{
            QStringLiteral("1012.40"),
            QStringLiteral("3120.40"),
            QStringLiteral("3188.10"),
            QStringLiteral("68120.50"),
            QStringLiteral("\"0.25\""),
            QStringLiteral("'0.25'"),
        };

        int scannedFiles = 0;
        while (it.hasNext()) {
            const QString filePath = it.next();
            ++scannedFiles;
            QFile f(filePath);
            QVERIFY(f.open(QIODevice::ReadOnly));
            const QString content = QString::fromUtf8(f.readAll());
            for (const QString& lit : forbiddenLiterals) {
                QVERIFY2(!content.contains(lit),
                         qPrintable(QStringLiteral("Forbidden hardcoded domain literal '%1' in %2")
                                        .arg(lit, filePath)));
            }
        }
        QVERIFY(scannedFiles >= 17); // Theme + 7 components + Main + 9 pages
    }
};

QTEST_MAIN(QmlSmokeTest)
#include "tst_qml_smoke.moc"
