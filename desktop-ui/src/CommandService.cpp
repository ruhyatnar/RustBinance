#include "CommandService.h"

#include <QDateTime>
#include <QJsonObject>

#include "IpcClient.h"
#include "StateStore.h"

namespace botui {

CommandService::CommandService(StateStore* store, IpcClient* client, QObject* parent)
    : QObject(parent), m_store(store), m_client(client) {
    if (m_client != nullptr) {
        connect(m_client, &IpcClient::commandResultReceived, this, &CommandService::onCommandResult);
    }
    if (m_store != nullptr) {
        connect(m_store, &StateStore::connectionChanged, this, [this]() {
            if (m_store->connectionState() != QStringLiteral("connected")) {
                const bool wasBusy = busy();
                m_pendingConfirms.clear();
                m_pendingSwitches.clear();
                m_trackedMutations.clear();
                if (wasBusy != busy()) {
                    emit busyChanged();
                }
            }
        });
        connect(m_store, &StateStore::securityChanged, this, [this]() {
            if (!m_store->canMutate() && !m_pendingConfirms.isEmpty()) {
                const bool wasBusy = busy();
                m_pendingConfirms.clear();
                m_pendingSwitches.clear();
                if (wasBusy != busy()) {
                    emit busyChanged();
                }
            }
        });
    }
}

bool CommandService::requireMutationUnlocked() {
    if (m_store == nullptr || m_client == nullptr) {
        return false;
    }
    if (!m_client->isSocketConnected()) {
        m_store->setActionFeedback(QStringLiteral("Daemon socket is not connected"));
        return false;
    }
    if (!m_store->canMutate()) {
        m_store->setActionFeedback(
            QStringLiteral("Controls are locked (View-Only). Click Unlock Controls first."));
        return false;
    }
    return true;
}

void CommandService::dispatchSimpleMutation(const QJsonObject& fields) {
    const bool wasBusy = busy();
    const QString cmdId = m_client->sendCommand(fields);
    if (cmdId.isEmpty()) {
        if (m_store != nullptr) {
            m_store->setActionFeedback(QStringLiteral("Failed to write command to UDS socket"));
        }
        return;
    }
    m_trackedMutations.insert(cmdId);
    if (wasBusy != busy()) {
        emit busyChanged();
    }
}

void CommandService::pause() {
    if (!requireMutationUnlocked()) {
        return;
    }
    dispatchSimpleMutation(QJsonObject{{QStringLiteral("cmd"), QStringLiteral("pause")}});
}

void CommandService::resume() {
    if (!requireMutationUnlocked()) {
        return;
    }
    dispatchSimpleMutation(QJsonObject{{QStringLiteral("cmd"), QStringLiteral("resume")}});
}

void CommandService::trayPause() {
    if (m_client == nullptr || !m_client->isSocketConnected()) {
        return;
    }
    dispatchSimpleMutation(QJsonObject{{QStringLiteral("cmd"), QStringLiteral("pause")}});
}

void CommandService::trayResume() {
    if (m_client == nullptr || !m_client->isSocketConnected()) {
        return;
    }
    dispatchSimpleMutation(QJsonObject{{QStringLiteral("cmd"), QStringLiteral("resume")}});
}

void CommandService::closePosition(const QString& positionId) {
    if (!requireMutationUnlocked()) {
        return;
    }
    const QString clean = positionId.trimmed();
    if (clean.isEmpty()) {
        return;
    }
    dispatchSimpleMutation(QJsonObject{
        {QStringLiteral("cmd"), QStringLiteral("close_position")},
        {QStringLiteral("position_id"), clean},
    });
}

void CommandService::setParam(const QString& key, const QString& value) {
    if (!requireMutationUnlocked()) {
        return;
    }
    const QString cleanKey = key.trimmed();
    const QString cleanVal = value.trimmed();
    if (cleanKey.isEmpty() || cleanVal.isEmpty()) {
        m_store->setActionFeedback(QStringLiteral("Parameter key and value must not be empty"));
        return;
    }
    dispatchSimpleMutation(QJsonObject{
        {QStringLiteral("cmd"), QStringLiteral("set_param")},
        {QStringLiteral("key"), cleanKey},
        {QStringLiteral("value"), cleanVal},
    });
}

void CommandService::disableStrategy(const QString& strategyId) {
    if (!requireMutationUnlocked()) {
        return;
    }
    const QString clean = strategyId.trimmed();
    if (clean.isEmpty()) {
        m_store->setActionFeedback(QStringLiteral("Strategy identifier must not be empty"));
        return;
    }
    dispatchSimpleMutation(QJsonObject{
        {QStringLiteral("cmd"), QStringLiteral("disable_strategy")},
        {QStringLiteral("id"), clean},
    });
}

void CommandService::ackAlert(const QString& alertId) {
    if (!requireMutationUnlocked()) {
        return;
    }
    const QString clean = alertId.trimmed();
    if (clean.isEmpty()) {
        return;
    }
    dispatchSimpleMutation(QJsonObject{
        {QStringLiteral("cmd"), QStringLiteral("ack_alert")},
        {QStringLiteral("id"), clean},
    });
}

void CommandService::executeConfirmedAction(const QString& action) {
    if (!requireMutationUnlocked()) {
        return;
    }
    if (action != QStringLiteral("panic_flatten") && action != QStringLiteral("reset_kill")) {
        m_store->setActionFeedback(QStringLiteral("Unsupported confirmed action: ") + action);
        return;
    }

    const bool wasBusy = busy();
    const QString reqCmdId = m_client->sendCommand(QJsonObject{
        {QStringLiteral("cmd"), QStringLiteral("request_confirm")},
        {QStringLiteral("action"), action},
    });
    if (reqCmdId.isEmpty()) {
        m_store->setActionFeedback(QStringLiteral("Failed to request confirmation nonce"));
        return;
    }
    m_pendingConfirms.insert(reqCmdId, action);
    if (wasBusy != busy()) {
        emit busyChanged();
    }
}

void CommandService::switchVenue(const QString& targetVenue, const QString& onOpenPositions) {
    if (!requireMutationUnlocked()) {
        return;
    }
    const QString target = targetVenue.trimmed().toLower();
    const QString policy = onOpenPositions.trimmed().toLower();
    if (target != QStringLiteral("spot") && target != QStringLiteral("futures")) {
        m_store->setActionFeedback(QStringLiteral("Unsupported target venue: ") + targetVenue);
        return;
    }
    if (policy != QStringLiteral("drain") && policy != QStringLiteral("refuse") &&
        policy != QStringLiteral("flatten")) {
        m_store->setActionFeedback(QStringLiteral("Unsupported switch policy: ") + onOpenPositions);
        return;
    }
    if (policy == QStringLiteral("flatten")) {
        const bool wasBusy = busy();
        const QString reqCmdId = m_client->sendCommand(QJsonObject{
            {QStringLiteral("cmd"), QStringLiteral("request_confirm")},
            {QStringLiteral("action"), QStringLiteral("switch_venue")},
        });
        if (reqCmdId.isEmpty()) {
            m_store->setActionFeedback(QStringLiteral("Failed to request switch_venue confirmation nonce"));
            return;
        }
        m_pendingConfirms.insert(reqCmdId, QStringLiteral("switch_venue"));
        m_pendingSwitches.insert(reqCmdId, qMakePair(target, policy));
        if (wasBusy != busy()) {
            emit busyChanged();
        }
        return;
    }
    dispatchSimpleMutation(QJsonObject{
        {QStringLiteral("cmd"), QStringLiteral("switch_venue")},
        {QStringLiteral("target"), target},
        {QStringLiteral("on_open_positions"), policy},
        {QStringLiteral("confirm_nonce"), QString()},
    });
}

void CommandService::refreshKlines(const QString& venue,
                                   const QString& symbol,
                                   const QString& tf,
                                   int limit) {
    if (m_client == nullptr || !m_client->isSocketConnected()) {
        return;
    }
    const QString cleanVenue = venue.trimmed().isEmpty() ? QStringLiteral("spot") : venue.trimmed();
    const QString cleanSym = symbol.trimmed();
    const QString cleanTf = tf.trimmed().isEmpty() ? QStringLiteral("15m") : tf.trimmed();
    if (cleanSym.isEmpty()) {
        return;
    }
    if (m_store != nullptr) {
        m_store->setChartSymbol(cleanSym);
        m_store->setChartTf(cleanTf);
    }
    const QString reqId = m_client->requestKlines(cleanVenue, cleanSym, cleanTf, limit);
    if (m_store != nullptr && !reqId.isEmpty()) {
        m_store->setExpectedKlinesReqId(reqId);
    }
}

void CommandService::refreshTrades(const QString& venue) {
    if (m_client == nullptr || !m_client->isSocketConnected()) {
        return;
    }
    m_client->requestTrades(venue);
}

void CommandService::onCommandResult(const QString& cmdId,
                                     bool ok,
                                     const QString& code,
                                     const QString& message,
                                     const QString& confirmNonce,
                                     qint64 expiresAtMs) {
    const bool wasBusy = busy();

    if (m_pendingConfirms.contains(cmdId)) {
        const QString action = m_pendingConfirms.take(cmdId);
        const auto pendingSwitch = m_pendingSwitches.take(cmdId);
        if (!requireMutationUnlocked()) {
            if (wasBusy != busy()) {
                emit busyChanged();
            }
            return;
        }
        if (!ok || confirmNonce.isEmpty()) {
            if (m_store != nullptr) {
                m_store->setActionFeedback(
                    QStringLiteral("Confirmation failed (%1): %2").arg(code, message));
            }
            if (wasBusy != busy()) {
                emit busyChanged();
            }
            return;
        }
        if (expiresAtMs > 0 && expiresAtMs <= QDateTime::currentMSecsSinceEpoch()) {
            if (m_store != nullptr) {
                m_store->setActionFeedback(QStringLiteral("Confirmation nonce expired before use"));
            }
            if (wasBusy != busy()) {
                emit busyChanged();
            }
            return;
        }

        QJsonObject payload{
            {QStringLiteral("cmd"), action},
            {QStringLiteral("confirm_nonce"), confirmNonce},
        };
        if (action == QStringLiteral("switch_venue")) {
            payload.insert(QStringLiteral("target"), pendingSwitch.first);
            payload.insert(QStringLiteral("on_open_positions"), pendingSwitch.second);
        }
        const QString execId = m_client->sendCommand(payload);
        if (!execId.isEmpty()) {
            m_trackedMutations.insert(execId);
        }
        if (wasBusy != busy()) {
            emit busyChanged();
        }
        return;
    }

    if (m_trackedMutations.remove(cmdId)) {
        if (m_store != nullptr) {
            if (ok) {
                m_store->setActionFeedback(message);
            } else {
                m_store->setActionFeedback(
                    QStringLiteral("Rejected [%1]: %2").arg(code, message));
            }
        }
        if (ok && m_client != nullptr) {
            m_client->requestSnapshot();
        }
    }

    if (wasBusy != busy()) {
        emit busyChanged();
    }
}

} // namespace botui
