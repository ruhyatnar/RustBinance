#include <QApplication>
#include <QCommandLineOption>
#include <QCommandLineParser>
#include <QQmlApplicationEngine>
#include <QQmlContext>
#include <QQuickWindow>
#include <QTimer>
#include <QtQml/qqmlextensionplugin.h>

#include "CommandService.h"
#include "DecimalFormat.h"
#include "IpcClient.h"
#include "Notifier.h"
#include "StateStore.h"

Q_IMPORT_QML_PLUGIN(BinanceBotPlugin)

int main(int argc, char* argv[]) {
    QApplication app(argc, argv);
    QApplication::setApplicationName(QStringLiteral("binance-bot-ui"));
    QApplication::setOrganizationName(QStringLiteral("binance-bot"));
    QApplication::setApplicationVersion(QStringLiteral("0.1.0"));

    QCommandLineParser parser;
    parser.setApplicationDescription(QStringLiteral("Binance Rust Trading Bot Desktop Cockpit (Qt6/QML)"));
    parser.addHelpOption();
    parser.addVersionOption();

    QCommandLineOption socketOpt(
        QStringLiteral("socket"),
        QStringLiteral("Path to binance-botd Unix domain socket"),
        QStringLiteral("path"));
    QCommandLineOption smokeMsOpt(
        QStringLiteral("smoke-test-ms"),
        QStringLiteral("Exit cleanly after N milliseconds (used by automated smoke checks)"),
        QStringLiteral("ms"));
    parser.addOption(socketOpt);
    parser.addOption(smokeMsOpt);
    parser.process(app);

    botui::StateStore stateStore;
    botui::IpcClient ipcClient(&stateStore);
    botui::CommandService commandService(&stateStore, &ipcClient);
    botui::DecimalFormat decimalFormat;
    botui::Notifier notifier(&stateStore, &commandService);

    app.setWindowIcon(botui::Notifier::renderModeIcon(stateStore.buildMode(), stateStore.engineMode()));
    app.setQuitOnLastWindowClosed(!notifier.hasSystemTray());

    QQmlApplicationEngine engine;
    engine.rootContext()->setContextProperty(QStringLiteral("stateStore"), &stateStore);
    engine.rootContext()->setContextProperty(QStringLiteral("ipcClient"), &ipcClient);
    engine.rootContext()->setContextProperty(QStringLiteral("commandService"), &commandService);
    engine.rootContext()->setContextProperty(QStringLiteral("decimalFormat"), &decimalFormat);
    engine.rootContext()->setContextProperty(QStringLiteral("notifier"), &notifier);

    const QUrl mainUrl(QStringLiteral("qrc:/qt/qml/BinanceBot/qml/Main.qml"));
    QObject::connect(
        &engine,
        &QQmlApplicationEngine::objectCreated,
        &app,
        [mainUrl](QObject* obj, const QUrl& objUrl) {
            if (obj == nullptr && mainUrl == objUrl) {
                QCoreApplication::exit(1);
            }
        },
        Qt::QueuedConnection);

    engine.load(mainUrl);
    if (engine.rootObjects().isEmpty()) {
        return 1;
    }

    auto* mainWindow = qobject_cast<QQuickWindow*>(engine.rootObjects().first());
    if (mainWindow != nullptr) {
        QObject::connect(&notifier, &botui::Notifier::openCockpitRequested, mainWindow, [mainWindow]() {
            mainWindow->show();
            mainWindow->raise();
            mainWindow->requestActivate();
        });
    }

    const QString sockPath = botui::IpcClient::resolveDefaultSocketPath(parser.value(socketOpt));
    ipcClient.connectToDaemon(sockPath);

    if (parser.isSet(smokeMsOpt)) {
        bool ok = false;
        const int exitMs = parser.value(smokeMsOpt).toInt(&ok);
        if (ok && exitMs >= 0) {
            QTimer::singleShot(exitMs, &app, &QCoreApplication::quit);
        }
    }

    return app.exec();
}
