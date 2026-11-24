#pragma once

#include <QHash>
#include <QObject>
#include <QSet>
#include <QString>

namespace botui {

class IpcClient;
class StateStore;

class CommandService : public QObject {
    Q_OBJECT
    Q_PROPERTY(bool busy READ busy NOTIFY busyChanged)

public:
    explicit CommandService(StateStore* store, IpcClient* client, QObject* parent = nullptr);

    bool busy() const { return !m_pendingConfirms.isEmpty() || !m_trackedMutations.isEmpty(); }

    Q_INVOKABLE void pause();
    Q_INVOKABLE void resume();
    Q_INVOKABLE void closePosition(const QString& positionId);
    Q_INVOKABLE void setParam(const QString& key, const QString& value);
    Q_INVOKABLE void disableStrategy(const QString& strategyId);
    Q_INVOKABLE void ackAlert(const QString& alertId);
    Q_INVOKABLE void executeConfirmedAction(const QString& action);
    Q_INVOKABLE void refreshKlines(const QString& venue,
                                   const QString& symbol,
                                   const QString& tf,
                                   int limit = 120);
    Q_INVOKABLE void refreshTrades(const QString& venue = QString());

    // Used exclusively by the system tray quick menu when connected (§11.4: Pause/Resume only, never Panic)
    void trayPause();
    void trayResume();

signals:
    void busyChanged();

private slots:
    void onCommandResult(const QString& cmdId,
                         bool ok,
                         const QString& code,
                         const QString& message,
                         const QString& confirmNonce,
                         qint64 expiresAtMs);

private:
    bool requireMutationUnlocked();
    void dispatchSimpleMutation(const QJsonObject& fields);

    StateStore* m_store = nullptr;
    IpcClient* m_client = nullptr;
    QHash<QString, QString> m_pendingConfirms; // cmd_id -> action ("panic_flatten" | "reset_kill")
    QSet<QString> m_trackedMutations;
};

} // namespace botui
