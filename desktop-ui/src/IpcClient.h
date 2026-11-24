#pragma once

#include <QByteArray>
#include <QJsonObject>
#include <QLocalSocket>
#include <QObject>
#include <QProcessEnvironment>
#include <QSet>
#include <QString>
#include <QTimer>

namespace botui {

class StateStore;

class IpcClient : public QObject {
    Q_OBJECT

public:
    explicit IpcClient(StateStore* store, QObject* parent = nullptr);

    static QString resolveDefaultSocketPath(
        const QString& cliSocket = QString(),
        const QProcessEnvironment& env = QProcessEnvironment::systemEnvironment());

    void setAllowedPeerUids(const QSet<quint32>& uids, const QSet<quint32>& gids = {});

    Q_INVOKABLE void connectToDaemon(const QString& socketPath);
    Q_INVOKABLE void disconnectFromDaemon();
    Q_INVOKABLE void requestSnapshot();

    QString sendCommand(const QJsonObject& commandKindFields);
    QString requestKlines(const QString& venue, const QString& symbol, const QString& tf, int limit);
    QString requestTrades(const QString& venue = QString(), qint64 fromMs = -1, qint64 toMs = -1);

    bool isSocketConnected() const;

signals:
    void commandResultReceived(const QString& cmdId,
                               bool ok,
                               const QString& code,
                               const QString& message,
                               const QString& confirmNonce,
                               qint64 expiresAtMs);

private slots:
    void onSocketConnected();
    void onSocketDisconnected();
    void onSocketError(QLocalSocket::LocalSocketError socketError);
    void onSocketReadyRead();
    void onReconnectTimeout();

private:
    bool verifyPeerCredentials(QString& reasonOut) const;
    bool sendClientEnvelope(const QString& type, const QJsonObject& payload);
    void scheduleReconnect();

    StateStore* m_store = nullptr;
    QLocalSocket* m_socket = nullptr;
    QTimer* m_reconnectTimer = nullptr;
    QString m_targetSocketPath;
    QByteArray m_readBuffer;
    quint64 m_clientSeq = 0;
    int m_reconnectAttempts = 0;
    bool m_manualDisconnect = true;
    bool m_peerVerified = false;
    QSet<quint32> m_allowedPeerUids;
    QSet<quint32> m_allowedPeerGids;
};

} // namespace botui
