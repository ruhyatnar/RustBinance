#include "StateStore.h"

#include <QJsonArray>

namespace botui {

StateStore::StateStore(QObject* parent)
    : QObject(parent),
      m_positionsModel(new PositionsModel(this)),
      m_ordersModel(new OrdersModel(this)),
      m_signalsModel(new SignalsModel(this)),
      m_marketTicksModel(new MarketTicksModel(this)),
      m_eventsModel(new EventsModel(this)),
      m_alertsModel(new AlertsModel(this)),
      m_tradesModel(new TradesModel(this)) {}

void StateStore::setConnectionState(const QString& state) {
    if (m_connectionState == state) {
        return;
    }
    const bool prevMutate = canMutate();
    m_connectionState = state;
    if (state != QStringLiteral("connected")) {
        // Re-lock mutating controls whenever session disconnects for safety.
        m_controlsUnlocked = false;
    }
    emit connectionChanged();
    if (prevMutate != canMutate()) {
        emit securityChanged();
    }
}

void StateStore::setSocketPath(const QString& path) {
    if (m_socketPath == path) {
        return;
    }
    m_socketPath = path;
    emit connectionChanged();
}

void StateStore::setLastError(const QString& err) {
    if (m_lastError == err) {
        return;
    }
    m_lastError = err;
    emit connectionChanged();
}

void StateStore::setControlsUnlocked(bool unlocked) {
    const bool next = (m_connectionState == QStringLiteral("connected")) ? unlocked : false;
    if (m_controlsUnlocked == next) {
        return;
    }
    m_controlsUnlocked = next;
    emit securityChanged();
}

bool StateStore::canMutate() const {
    return m_connectionState == QStringLiteral("connected") && m_controlsUnlocked;
}

void StateStore::setActionFeedback(const QString& msg) {
    if (m_actionFeedback == msg) {
        return;
    }
    m_actionFeedback = msg;
    emit feedbackChanged();
}

void StateStore::setChartSymbol(const QString& symbol) {
    const QString clean = symbol.trimmed();
    if (clean.isEmpty() || m_chartSymbol == clean) {
        return;
    }
    m_chartSymbol = clean;
    m_userChoseChartSymbol = true;
    emit chartChanged();
}

void StateStore::setChartTf(const QString& tf) {
    const QString clean = tf.trimmed();
    if (clean.isEmpty() || m_chartTf == clean) {
        return;
    }
    m_chartTf = clean;
    emit chartChanged();
}

QVariantMap StateStore::activeChartPosition() const {
    return m_positionsModel->firstPositionForSymbol(m_chartSymbol);
}

QStringList StateStore::availableSymbols() const {
    QStringList syms = m_marketTicksModel->symbols();
    const QString posSym = m_positionsModel->firstSymbolOrDefault(QString());
    if (!posSym.isEmpty() && !syms.contains(posSym)) {
        syms.prepend(posSym);
    }
    if (!m_chartSymbol.isEmpty() && !syms.contains(m_chartSymbol)) {
        syms.append(m_chartSymbol);
    }
    return syms;
}

void StateStore::resetSessionSeq() {
    m_hasSeq = false;
    m_lastSeq = 0;
}

EnvelopeOutcome StateStore::applyEnvelope(const QJsonObject& env) {
    if (env.value(QStringLiteral("v")).toInt() != 1) {
        return EnvelopeOutcome::Ignored;
    }
    const QString type = env.value(QStringLiteral("type")).toString();
    const QJsonValue seqVal = env.value(QStringLiteral("seq"));
    const QJsonValue payloadVal = env.value(QStringLiteral("payload"));
    if (type.isEmpty() || !seqVal.isDouble() || !payloadVal.isObject()) {
        return EnvelopeOutcome::Ignored;
    }

    const qint64 signedSeq = seqVal.toInteger(-1);
    if (signedSeq < 0) {
        return EnvelopeOutcome::Ignored;
    }
    const auto seq = static_cast<quint64>(signedSeq);
    const qint64 tsMs = env.value(QStringLiteral("ts_ms")).toInteger();
    const QJsonObject payload = payloadVal.toObject();

    // Sequence continuity check (§10.2): hello and full snapshot establish/resync baseline.
    if (type == QStringLiteral("hello") || type == QStringLiteral("snapshot")) {
        m_lastSeq = seq;
        m_hasSeq = true;
    } else if (m_hasSeq) {
        if (seq != m_lastSeq + 1) {
            emit seqGapDetected(m_lastSeq + 1, seq);
            return EnvelopeOutcome::SeqGap;
        }
        m_lastSeq = seq;
    } else {
        m_lastSeq = seq;
        m_hasSeq = true;
    }

    if (type == QStringLiteral("hello")) {
        m_protocolVersion = payload.value(QStringLiteral("protocol_version")).toInt(1);
        const QString bm = payload.value(QStringLiteral("build_mode")).toString();
        if (!bm.isEmpty()) {
            m_buildMode = bm;
        }
        m_engineInstanceId = payload.value(QStringLiteral("engine_instance_id")).toString();
        emit helloChanged();
        emit snapshotChanged();
        emit telemetryChanged();
        return EnvelopeOutcome::Ok;
    }

    if (type == QStringLiteral("snapshot")) {
        applyVenuesObj(payload.value(QStringLiteral("venues")).toObject());
        applyEngineObj(payload.value(QStringLiteral("engine")).toObject());
        applyRiskObj(payload.value(QStringLiteral("risk")).toObject());
        m_positionsModel->replaceFromJson(payload.value(QStringLiteral("positions")).toArray());
        m_ordersModel->replaceFromJson(payload.value(QStringLiteral("orders")).toArray());
        m_signalsModel->replaceFromJson(payload.value(QStringLiteral("signals_shadow")).toArray());
        m_alertsModel->replaceFromJson(payload.value(QStringLiteral("alerts")).toArray());
        m_hasSnapshot = true;

        if (!m_userChoseChartSymbol) {
            m_chartSymbol = m_positionsModel->firstSymbolOrDefault(m_chartSymbol);
        }

        notifyNewAlerts();
        emit snapshotChanged();
        emit telemetryChanged();
        emit chartChanged();
        return EnvelopeOutcome::Ok;
    }

    if (type == QStringLiteral("delta")) {
        if (payload.value(QStringLiteral("venues")).isObject()) {
            applyVenuesObj(payload.value(QStringLiteral("venues")).toObject());
        }
        if (payload.value(QStringLiteral("engine")).isObject()) {
            applyEngineObj(payload.value(QStringLiteral("engine")).toObject());
        }
        if (payload.value(QStringLiteral("risk")).isObject()) {
            applyRiskObj(payload.value(QStringLiteral("risk")).toObject());
        }
        if (payload.value(QStringLiteral("positions")).isArray()) {
            m_positionsModel->replaceFromJson(payload.value(QStringLiteral("positions")).toArray());
        }
        if (payload.value(QStringLiteral("orders")).isArray()) {
            m_ordersModel->replaceFromJson(payload.value(QStringLiteral("orders")).toArray());
        }
        if (payload.value(QStringLiteral("alerts")).isArray()) {
            m_alertsModel->replaceFromJson(payload.value(QStringLiteral("alerts")).toArray());
            notifyNewAlerts();
        }
        emit snapshotChanged();
        emit telemetryChanged();
        emit chartChanged();
        return EnvelopeOutcome::Ok;
    }

    if (type == QStringLiteral("market")) {
        m_marketTicksModel->replaceFromJson(payload.value(QStringLiteral("ticks")).toArray());
        emit telemetryChanged();
        return EnvelopeOutcome::Ok;
    }

    if (type == QStringLiteral("event")) {
        m_eventsModel->prependEvent(seq, tsMs, payload);
        const QString level = payload.value(QStringLiteral("level")).toString();
        const QString kind = payload.value(QStringLiteral("kind")).toString();
        const QString msg = payload.value(QStringLiteral("message")).toString();
        if (level == QStringLiteral("warn") || level == QStringLiteral("error") ||
            level == QStringLiteral("critical") || kind == QStringLiteral("Fill")) {
            emit criticalNotificationRequested(
                QStringLiteral("[%1] %2").arg(level.toUpper(), kind),
                msg,
                level);
        }
        emit telemetryChanged();
        return EnvelopeOutcome::Ok;
    }

    if (type == QStringLiteral("heartbeat")) {
        updateEngineModeAndNotify(payload.value(QStringLiteral("engine_state")).toString());
        m_wsLatencyMs = payload.value(QStringLiteral("ws_latency_ms")).toInteger();
        m_weightUsed = payload.value(QStringLiteral("weight_used")).toInt();
        m_weightLimit = payload.value(QStringLiteral("weight_limit")).toInt();
        emit snapshotChanged();
        emit telemetryChanged();
        return EnvelopeOutcome::Ok;
    }

    if (type == QStringLiteral("klines")) {
        const QJsonArray arr = payload.value(QStringLiteral("klines")).toArray();
        QVariantList next;
        next.reserve(arr.size());
        for (const QJsonValue& val : arr) {
            const QJsonObject b = val.toObject();
            next.push_back(QVariantMap{
                {QStringLiteral("time"), b.value(QStringLiteral("time")).toInteger()},
                {QStringLiteral("open"), b.value(QStringLiteral("open")).toString()},
                {QStringLiteral("high"), b.value(QStringLiteral("high")).toString()},
                {QStringLiteral("low"), b.value(QStringLiteral("low")).toString()},
                {QStringLiteral("close"), b.value(QStringLiteral("close")).toString()},
                {QStringLiteral("volume"), b.value(QStringLiteral("volume")).toString()},
            });
        }
        m_klines = next;
        emit klinesChanged();
        emit telemetryChanged();
        return EnvelopeOutcome::Ok;
    }

    if (type == QStringLiteral("trades")) {
        m_tradesModel->replaceFromJson(payload.value(QStringLiteral("trades")).toArray());
        emit telemetryChanged();
        return EnvelopeOutcome::Ok;
    }

    if (type == QStringLiteral("command_result")) {
        emit telemetryChanged();
        return EnvelopeOutcome::Ok;
    }

    return EnvelopeOutcome::Ignored;
}

void StateStore::applyVenuesObj(const QJsonObject& venues) {
    if (venues.isEmpty()) {
        return;
    }
    m_activeVenue = venues.value(QStringLiteral("active")).toString(m_activeVenue);
    m_spotState = venues.value(QStringLiteral("spot")).toString(m_spotState);
    m_futuresState = venues.value(QStringLiteral("futures")).toString(m_futuresState);
    m_switchesToday = venues.value(QStringLiteral("switches_today")).toInt(m_switchesToday);
}

void StateStore::applyEngineObj(const QJsonObject& engine) {
    if (engine.isEmpty()) {
        return;
    }
    updateEngineModeAndNotify(engine.value(QStringLiteral("mode")).toString(m_engineMode));
    m_buildMode = engine.value(QStringLiteral("build_mode")).toString(m_buildMode);
    m_uptimeS = engine.value(QStringLiteral("uptime_s")).toInteger(m_uptimeS);
    m_wsLatencyMs = engine.value(QStringLiteral("ws_latency_ms")).toInteger(m_wsLatencyMs);
    m_clockDriftMs = engine.value(QStringLiteral("clock_drift_ms")).toInteger(m_clockDriftMs);
    const QJsonObject rate = engine.value(QStringLiteral("rate")).toObject();
    if (!rate.isEmpty()) {
        m_weightUsed = rate.value(QStringLiteral("weight_used")).toInt(m_weightUsed);
        m_weightLimit = rate.value(QStringLiteral("weight_limit")).toInt(m_weightLimit);
        m_orders10s = rate.value(QStringLiteral("orders_10s")).toInt(m_orders10s);
    }
}

void StateStore::applyRiskObj(const QJsonObject& risk) {
    if (risk.isEmpty()) {
        return;
    }
    m_equity = risk.value(QStringLiteral("equity")).toString(m_equity);
    m_dayPnlPct = risk.value(QStringLiteral("day_pnl_pct")).toString(m_dayPnlPct);
    m_weekPnlPct = risk.value(QStringLiteral("week_pnl_pct")).toString(m_weekPnlPct);
    m_drawdownPct = risk.value(QStringLiteral("drawdown_pct")).toString(m_drawdownPct);
    m_heatPct = risk.value(QStringLiteral("heat_pct")).toString(m_heatPct);
    m_heatMaxPct = risk.value(QStringLiteral("heat_max_pct")).toString(m_heatMaxPct);
    m_openPositions = risk.value(QStringLiteral("open_positions")).toInt(m_openPositions);
    m_maxPositions = risk.value(QStringLiteral("max_positions")).toInt(m_maxPositions);
    m_btcGuard = risk.value(QStringLiteral("btc_guard")).toString(m_btcGuard);
    const QJsonObject limits = risk.value(QStringLiteral("limits")).toObject();
    if (!limits.isEmpty()) {
        m_dailyHaltPct = limits.value(QStringLiteral("daily_halt_pct")).toString(m_dailyHaltPct);
        m_killPct = limits.value(QStringLiteral("kill_pct")).toString(m_killPct);
    }
}

void StateStore::notifyNewAlerts() {
    for (const AlertItem& alert : m_alertsModel->items()) {
        if (!alert.acked && !alert.id.isEmpty() && !m_notifiedAlertIds.contains(alert.id)) {
            m_notifiedAlertIds.insert(alert.id);
            emit criticalNotificationRequested(
                QStringLiteral("Alert [%1]").arg(alert.level.toUpper()),
                alert.msg,
                alert.level);
        }
    }
}

void StateStore::updateEngineModeAndNotify(const QString& nextMode) {
    if (nextMode.isEmpty() || m_engineMode == nextMode) {
        return;
    }
    const QString prev = m_engineMode;
    m_engineMode = nextMode;
    if ((nextMode == QStringLiteral("Halted") || nextMode == QStringLiteral("Killed")) &&
        prev != nextMode) {
        emit criticalNotificationRequested(
            QStringLiteral("Engine %1").arg(nextMode.toUpper()),
            QStringLiteral("Trading engine transitioned from %1 to %2").arg(prev, nextMode),
            QStringLiteral("critical"));
    }
}

} // namespace botui
