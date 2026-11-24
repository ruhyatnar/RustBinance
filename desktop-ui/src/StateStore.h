#pragma once

#include <QJsonObject>
#include <QObject>
#include <QSet>
#include <QString>
#include <QVariantList>
#include <QVariantMap>

#include "Models.h"

namespace botui {

enum class EnvelopeOutcome {
    Ok,
    SeqGap,
    Ignored,
};

class StateStore : public QObject {
    Q_OBJECT

    // Connection & lock state
    Q_PROPERTY(QString connectionState READ connectionState WRITE setConnectionState NOTIFY connectionChanged)
    Q_PROPERTY(QString socketPath READ socketPath WRITE setSocketPath NOTIFY connectionChanged)
    Q_PROPERTY(QString lastError READ lastError WRITE setLastError NOTIFY connectionChanged)
    Q_PROPERTY(qulonglong lastSeq READ lastSeq NOTIFY telemetryChanged)
    Q_PROPERTY(bool hasSnapshot READ hasSnapshot NOTIFY snapshotChanged)
    Q_PROPERTY(bool controlsUnlocked READ controlsUnlocked WRITE setControlsUnlocked NOTIFY securityChanged)
    Q_PROPERTY(bool canMutate READ canMutate NOTIFY securityChanged)
    Q_PROPERTY(QString actionFeedback READ actionFeedback WRITE setActionFeedback NOTIFY feedbackChanged)

    // Hello & Engine summary
    Q_PROPERTY(int protocolVersion READ protocolVersion NOTIFY helloChanged)
    Q_PROPERTY(QString buildMode READ buildMode NOTIFY snapshotChanged)
    Q_PROPERTY(QString engineInstanceId READ engineInstanceId NOTIFY helloChanged)
    Q_PROPERTY(QString engineMode READ engineMode NOTIFY snapshotChanged)
    Q_PROPERTY(qint64 uptimeS READ uptimeS NOTIFY snapshotChanged)
    Q_PROPERTY(qint64 wsLatencyMs READ wsLatencyMs NOTIFY telemetryChanged)
    Q_PROPERTY(qint64 clockDriftMs READ clockDriftMs NOTIFY snapshotChanged)
    Q_PROPERTY(int weightUsed READ weightUsed NOTIFY telemetryChanged)
    Q_PROPERTY(int weightLimit READ weightLimit NOTIFY telemetryChanged)
    Q_PROPERTY(int orders10s READ orders10s NOTIFY snapshotChanged)

    // Venues summary
    Q_PROPERTY(QString activeVenue READ activeVenue NOTIFY snapshotChanged)
    Q_PROPERTY(QString spotState READ spotState NOTIFY snapshotChanged)
    Q_PROPERTY(QString futuresState READ futuresState NOTIFY snapshotChanged)
    Q_PROPERTY(int switchesToday READ switchesToday NOTIFY snapshotChanged)

    // Risk summary (all exact DecimalStrings)
    Q_PROPERTY(QString equity READ equity NOTIFY snapshotChanged)
    Q_PROPERTY(QString dayPnlPct READ dayPnlPct NOTIFY snapshotChanged)
    Q_PROPERTY(QString weekPnlPct READ weekPnlPct NOTIFY snapshotChanged)
    Q_PROPERTY(QString drawdownPct READ drawdownPct NOTIFY snapshotChanged)
    Q_PROPERTY(QString heatPct READ heatPct NOTIFY snapshotChanged)
    Q_PROPERTY(QString heatMaxPct READ heatMaxPct NOTIFY snapshotChanged)
    Q_PROPERTY(int openPositions READ openPositions NOTIFY snapshotChanged)
    Q_PROPERTY(int maxPositions READ maxPositions NOTIFY snapshotChanged)
    Q_PROPERTY(QString btcGuard READ btcGuard NOTIFY snapshotChanged)
    Q_PROPERTY(QString dailyHaltPct READ dailyHaltPct NOTIFY snapshotChanged)
    Q_PROPERTY(QString killPct READ killPct NOTIFY snapshotChanged)
    Q_PROPERTY(int unackedAlertCount READ unackedAlertCount NOTIFY snapshotChanged)

    // Chart state & klines
    Q_PROPERTY(QString chartSymbol READ chartSymbol WRITE setChartSymbol NOTIFY chartChanged)
    Q_PROPERTY(QString chartTf READ chartTf WRITE setChartTf NOTIFY chartChanged)
    Q_PROPERTY(QVariantList klines READ klines NOTIFY klinesChanged)
    Q_PROPERTY(QVariantMap activeChartPosition READ activeChartPosition NOTIFY chartChanged)
    Q_PROPERTY(QStringList availableSymbols READ availableSymbols NOTIFY telemetryChanged)

    // List models
    Q_PROPERTY(botui::PositionsModel* positionsModel READ positionsModel CONSTANT)
    Q_PROPERTY(botui::OrdersModel* ordersModel READ ordersModel CONSTANT)
    Q_PROPERTY(botui::SignalsModel* signalsModel READ signalsModel CONSTANT)
    Q_PROPERTY(botui::MarketTicksModel* marketTicksModel READ marketTicksModel CONSTANT)
    Q_PROPERTY(botui::EventsModel* eventsModel READ eventsModel CONSTANT)
    Q_PROPERTY(botui::AlertsModel* alertsModel READ alertsModel CONSTANT)
    Q_PROPERTY(botui::TradesModel* tradesModel READ tradesModel CONSTANT)

public:
    explicit StateStore(QObject* parent = nullptr);

    QString connectionState() const { return m_connectionState; }
    void setConnectionState(const QString& state);

    QString socketPath() const { return m_socketPath; }
    void setSocketPath(const QString& path);

    QString lastError() const { return m_lastError; }
    void setLastError(const QString& err);

    qulonglong lastSeq() const { return m_lastSeq; }
    bool hasSnapshot() const { return m_hasSnapshot; }

    bool controlsUnlocked() const { return m_controlsUnlocked; }
    Q_INVOKABLE void setControlsUnlocked(bool unlocked);
    Q_INVOKABLE void toggleControlsUnlocked() { setControlsUnlocked(!m_controlsUnlocked); }
    bool canMutate() const;

    QString actionFeedback() const { return m_actionFeedback; }
    void setActionFeedback(const QString& msg);
    Q_INVOKABLE void clearActionFeedback() { setActionFeedback(QString()); }

    int protocolVersion() const { return m_protocolVersion; }
    QString buildMode() const { return m_buildMode; }
    QString engineInstanceId() const { return m_engineInstanceId; }
    QString engineMode() const { return m_engineMode; }
    qint64 uptimeS() const { return m_uptimeS; }
    qint64 wsLatencyMs() const { return m_wsLatencyMs; }
    qint64 clockDriftMs() const { return m_clockDriftMs; }
    int weightUsed() const { return m_weightUsed; }
    int weightLimit() const { return m_weightLimit; }
    int orders10s() const { return m_orders10s; }

    QString activeVenue() const { return m_activeVenue; }
    QString spotState() const { return m_spotState; }
    QString futuresState() const { return m_futuresState; }
    int switchesToday() const { return m_switchesToday; }

    QString equity() const { return m_equity; }
    QString dayPnlPct() const { return m_dayPnlPct; }
    QString weekPnlPct() const { return m_weekPnlPct; }
    QString drawdownPct() const { return m_drawdownPct; }
    QString heatPct() const { return m_heatPct; }
    QString heatMaxPct() const { return m_heatMaxPct; }
    int openPositions() const { return m_openPositions; }
    int maxPositions() const { return m_maxPositions; }
    QString btcGuard() const { return m_btcGuard; }
    QString dailyHaltPct() const { return m_dailyHaltPct; }
    QString killPct() const { return m_killPct; }
    int unackedAlertCount() const { return m_alertsModel->unackedCount(); }

    QString chartSymbol() const { return m_chartSymbol; }
    void setChartSymbol(const QString& symbol);

    QString chartTf() const { return m_chartTf; }
    void setChartTf(const QString& tf);

    QVariantList klines() const { return m_klines; }
    QVariantMap activeChartPosition() const;
    QStringList availableSymbols() const;

    PositionsModel* positionsModel() const { return m_positionsModel; }
    OrdersModel* ordersModel() const { return m_ordersModel; }
    SignalsModel* signalsModel() const { return m_signalsModel; }
    MarketTicksModel* marketTicksModel() const { return m_marketTicksModel; }
    EventsModel* eventsModel() const { return m_eventsModel; }
    AlertsModel* alertsModel() const { return m_alertsModel; }
    TradesModel* tradesModel() const { return m_tradesModel; }

    EnvelopeOutcome applyEnvelope(const QJsonObject& env);
    void resetSessionSeq();

signals:
    void connectionChanged();
    void securityChanged();
    void feedbackChanged();
    void helloChanged();
    void snapshotChanged();
    void telemetryChanged();
    void chartChanged();
    void klinesChanged();
    void seqGapDetected(quint64 expected, quint64 got);
    void criticalNotificationRequested(const QString& title, const QString& body, const QString& level);

private:
    void applyVenuesObj(const QJsonObject& venues);
    void applyEngineObj(const QJsonObject& engine);
    void applyRiskObj(const QJsonObject& risk);
    void notifyNewAlerts();
    void updateEngineModeAndNotify(const QString& nextMode);

    QString m_connectionState = QStringLiteral("disconnected");
    QString m_socketPath;
    QString m_lastError;
    bool m_hasSeq = false;
    qulonglong m_lastSeq = 0;
    bool m_hasSnapshot = false;
    bool m_controlsUnlocked = false;
    QString m_actionFeedback;

    int m_protocolVersion = 1;
    QString m_buildMode = QStringLiteral("paper");
    QString m_engineInstanceId;
    QString m_engineMode = QStringLiteral("Starting");
    qint64 m_uptimeS = 0;
    qint64 m_wsLatencyMs = 0;
    qint64 m_clockDriftMs = 0;
    int m_weightUsed = 0;
    int m_weightLimit = 0;
    int m_orders10s = 0;

    QString m_activeVenue = QStringLiteral("spot");
    QString m_spotState = QStringLiteral("Active");
    QString m_futuresState = QStringLiteral("Disabled");
    int m_switchesToday = 0;

    QString m_equity;
    QString m_dayPnlPct;
    QString m_weekPnlPct;
    QString m_drawdownPct;
    QString m_heatPct;
    QString m_heatMaxPct;
    int m_openPositions = 0;
    int m_maxPositions = 0;
    QString m_btcGuard;
    QString m_dailyHaltPct;
    QString m_killPct;

    QString m_chartSymbol = QStringLiteral("BTCUSDT");
    bool m_userChoseChartSymbol = false;
    QString m_chartTf = QStringLiteral("15m");
    QVariantList m_klines;

    QSet<QString> m_notifiedAlertIds;

    PositionsModel* m_positionsModel = nullptr;
    OrdersModel* m_ordersModel = nullptr;
    SignalsModel* m_signalsModel = nullptr;
    MarketTicksModel* m_marketTicksModel = nullptr;
    EventsModel* m_eventsModel = nullptr;
    AlertsModel* m_alertsModel = nullptr;
    TradesModel* m_tradesModel = nullptr;
};

} // namespace botui
