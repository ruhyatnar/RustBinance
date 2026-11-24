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
    m_client->requestKlines(cleanVenue, cleanSym, cleanTf, limit);
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

        const QString execId = m_client->sendCommand(QJsonObject{
            {QStringLiteral("cmd"), action},
            {QStringLiteral("confirm_nonce"), confirmNonce},
        });
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
