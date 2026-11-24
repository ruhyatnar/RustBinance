#include "IpcClient.h"

#include <QDateTime>
#include <QFileInfo>
#include <QJsonDocument>
#include <QUuid>

#ifdef __linux__
#include <grp.h>
#include <sys/socket.h>
#include <sys/types.h>
#include <unistd.h>
#endif

#include "StateStore.h"

namespace botui {

namespace {
constexpr int kBaseBackoffMs = 250;
constexpr int kMaxBackoffMs = 5000;
} // namespace

IpcClient::IpcClient(StateStore* store, QObject* parent)
    : QObject(parent),
      m_store(store),
      m_socket(new QLocalSocket(this)),
      m_reconnectTimer(new QTimer(this)) {
#ifdef __linux__
    m_allowedPeerUids.insert(static_cast<quint32>(::geteuid()));
    m_allowedPeerUids.insert(0U);
    if (struct group* grp = ::getgrnam("binancebot"); grp != nullptr) {
        m_allowedPeerGids.insert(static_cast<quint32>(grp->gr_gid));
    }
#endif

    m_reconnectTimer->setSingleShot(true);
    connect(m_reconnectTimer, &QTimer::timeout, this, &IpcClient::onReconnectTimeout);
    connect(m_socket, &QLocalSocket::connected, this, &IpcClient::onSocketConnected);
    connect(m_socket, &QLocalSocket::disconnected, this, &IpcClient::onSocketDisconnected);
    connect(m_socket, &QLocalSocket::errorOccurred, this, &IpcClient::onSocketError);
    connect(m_socket, &QLocalSocket::readyRead, this, &IpcClient::onSocketReadyRead);

    if (m_store != nullptr) {
        connect(m_store, &StateStore::seqGapDetected, this, [this](quint64, quint64) {
            requestSnapshot();
        });
    }
}

QString IpcClient::resolveDefaultSocketPath(const QString& cliSocket,
                                            const QProcessEnvironment& env) {
    const QString cleanCli = cliSocket.trimmed();
    if (!cleanCli.isEmpty()) {
        return cleanCli;
    }

    const QString envSock = env.value(QStringLiteral("BINANCE_BOT_SOCKET")).trimmed();
    if (!envSock.isEmpty()) {
        return envSock;
    }

    const QString systemSock = QStringLiteral("/run/binance-bot/bot.sock");
    if (QFileInfo::exists(systemSock)) {
        return systemSock;
    }

    const QString xdgRuntime = env.value(QStringLiteral("XDG_RUNTIME_DIR")).trimmed();
    if (!xdgRuntime.isEmpty() && !xdgRuntime.startsWith(QStringLiteral("/tmp"))) {
        return xdgRuntime + QStringLiteral("/binance-bot/bot.sock");
    }

    return systemSock;
}

void IpcClient::setAllowedPeerUids(const QSet<quint32>& uids, const QSet<quint32>& gids) {
    m_allowedPeerUids = uids;
    m_allowedPeerGids = gids;
}

void IpcClient::connectToDaemon(const QString& socketPath) {
    const QString clean = socketPath.trimmed();
    if (clean.isEmpty()) {
        return;
    }

    m_manualDisconnect = false;
    m_reconnectTimer->stop();
    m_reconnectAttempts = 0;
    m_targetSocketPath = clean;
    m_readBuffer.clear();
    m_peerVerified = false;

    if (m_store != nullptr) {
        m_store->setSocketPath(clean);
        m_store->setLastError(QString());
        m_store->resetSessionSeq();
        m_store->setConnectionState(QStringLiteral("connecting"));
    }

    m_socket->abort();
    m_socket->connectToServer(m_targetSocketPath);
}

void IpcClient::disconnectFromDaemon() {
    m_manualDisconnect = true;
    m_reconnectTimer->stop();
    m_peerVerified = false;
    m_readBuffer.clear();
    m_socket->abort();
    if (m_store != nullptr) {
        m_store->resetSessionSeq();
        m_store->setConnectionState(QStringLiteral("disconnected"));
    }
}

bool IpcClient::isSocketConnected() const {
    return m_peerVerified && m_socket->state() == QLocalSocket::ConnectedState;
}

void IpcClient::requestSnapshot() {
    sendClientEnvelope(QStringLiteral("request_snapshot"), QJsonObject{});
}

QString IpcClient::sendCommand(const QJsonObject& commandKindFields) {
    const QString cmdId = QUuid::createUuid().toString(QUuid::WithoutBraces);
    QJsonObject payload = commandKindFields;
    payload.insert(QStringLiteral("cmd_id"), cmdId);
    if (!sendClientEnvelope(QStringLiteral("command"), payload)) {
        return QString();
    }
    return cmdId;
}

QString IpcClient::requestKlines(const QString& venue,
                                 const QString& symbol,
                                 const QString& tf,
                                 int limit) {
    const QString reqId = QUuid::createUuid().toString(QUuid::WithoutBraces);
    QJsonObject payload{
        {QStringLiteral("req_id"), reqId},
        {QStringLiteral("venue"), venue},
        {QStringLiteral("symbol"), symbol},
        {QStringLiteral("tf"), tf},
        {QStringLiteral("limit"), qBound(1, limit, 1000)},
    };
    if (!sendClientEnvelope(QStringLiteral("get_klines"), payload)) {
        return QString();
    }
    return reqId;
}

QString IpcClient::requestTrades(const QString& venue, qint64 fromMs, qint64 toMs) {
    const QString reqId = QUuid::createUuid().toString(QUuid::WithoutBraces);
    QJsonObject payload{
        {QStringLiteral("req_id"), reqId},
    };
    if (!venue.trimmed().isEmpty()) {
        payload.insert(QStringLiteral("venue"), venue.trimmed());
    }
    if (fromMs >= 0) {
        payload.insert(QStringLiteral("from_ms"), fromMs);
    }
    if (toMs >= 0) {
        payload.insert(QStringLiteral("to_ms"), toMs);
    }
    if (!sendClientEnvelope(QStringLiteral("get_trades"), payload)) {
        return QString();
    }
    return reqId;
}

bool IpcClient::verifyPeerCredentials(QString& reasonOut) const {
#ifdef __linux__
    const qintptr fd = m_socket->socketDescriptor();
    if (fd < 0) {
        reasonOut = QStringLiteral("Invalid UDS socket descriptor");
        return false;
    }
    struct ucred cred {};
    socklen_t len = sizeof(cred);
    if (::getsockopt(static_cast<int>(fd), SOL_SOCKET, SO_PEERCRED, &cred, &len) != 0) {
        reasonOut = QStringLiteral("Failed to read SO_PEERCRED on UDS socket");
        return false;
    }
    const auto peerUid = static_cast<quint32>(cred.uid);
    const auto peerGid = static_cast<quint32>(cred.gid);
    if (m_allowedPeerUids.contains(peerUid) || m_allowedPeerGids.contains(peerGid)) {
        return true;
    }
    reasonOut = QStringLiteral("Untrusted UDS peer credentials rejected (uid=%1, gid=%2)")
                    .arg(peerUid)
                    .arg(peerGid);
    return false;
#else
    Q_UNUSED(reasonOut);
    return true;
#endif
}

bool IpcClient::sendClientEnvelope(const QString& type, const QJsonObject& payload) {
    if (!isSocketConnected()) {
        return false;
    }
    ++m_clientSeq;
    const QJsonObject env{
        {QStringLiteral("v"), 1},
        {QStringLiteral("type"), type},
        {QStringLiteral("seq"), static_cast<qint64>(m_clientSeq)},
        {QStringLiteral("ts_ms"), QDateTime::currentMSecsSinceEpoch()},
        {QStringLiteral("payload"), payload},
    };
    QByteArray line = QJsonDocument(env).toJson(QJsonDocument::Compact);
    line.append('\n');
    const qint64 written = m_socket->write(line);
    m_socket->flush();
    return written == line.size();
}

void IpcClient::onSocketConnected() {
    QString rejectReason;
    if (!verifyPeerCredentials(rejectReason)) {
        m_peerVerified = false;
        if (m_store != nullptr) {
            m_store->setLastError(rejectReason);
        }
        m_socket->abort();
        scheduleReconnect();
        return;
    }

    m_peerVerified = true;
    m_readBuffer.clear();
    if (m_store != nullptr) {
        m_store->resetSessionSeq();
        m_store->setLastError(QString());
        m_store->setConnectionState(QStringLiteral("connected"));
    }

    if (m_socket->bytesAvailable() > 0) {
        onSocketReadyRead();
    }
}

void IpcClient::onSocketDisconnected() {
    m_peerVerified = false;
    m_readBuffer.clear();
    if (!m_manualDisconnect) {
        scheduleReconnect();
    }
}

void IpcClient::onSocketError(QLocalSocket::LocalSocketError) {
    m_peerVerified = false;
    if (m_store != nullptr && m_store->lastError().isEmpty()) {
        m_store->setLastError(m_socket->errorString());
    }
    if (!m_manualDisconnect) {
        scheduleReconnect();
    }
}

void IpcClient::onSocketReadyRead() {
    if (!m_peerVerified) {
        return;
    }
    m_readBuffer.append(m_socket->readAll());

    while (true) {
        const int nl = m_readBuffer.indexOf('\n');
        if (nl < 0) {
            break;
        }
        const QByteArray rawLine = m_readBuffer.left(nl).trimmed();
        m_readBuffer.remove(0, nl + 1);
        if (rawLine.isEmpty()) {
            continue;
        }

        QJsonParseError parseErr{};
        const QJsonDocument doc = QJsonDocument::fromJson(rawLine, &parseErr);
        if (parseErr.error != QJsonParseError::NoError || !doc.isObject()) {
            continue;
        }
        const QJsonObject obj = doc.object();
        const QString type = obj.value(QStringLiteral("type")).toString();
        if (type == QStringLiteral("hello")) {
            m_reconnectAttempts = 0;
        }

        if (m_store != nullptr) {
            m_store->applyEnvelope(obj);
        }

        if (type == QStringLiteral("command_result")) {
            const QJsonObject p = obj.value(QStringLiteral("payload")).toObject();
            emit commandResultReceived(
                p.value(QStringLiteral("cmd_id")).toString(),
                p.value(QStringLiteral("ok")).toBool(false),
                p.value(QStringLiteral("code")).toString(),
                p.value(QStringLiteral("message")).toString(),
                p.value(QStringLiteral("confirm_nonce")).toString(),
                p.value(QStringLiteral("expires_at_ms")).toInteger(0));
        }
    }
}

void IpcClient::scheduleReconnect() {
    if (m_manualDisconnect || m_targetSocketPath.isEmpty()) {
        return;
    }
    if (m_store != nullptr) {
        m_store->setConnectionState(QStringLiteral("reconnecting"));
    }
    if (m_reconnectTimer->isActive()) {
        return;
    }
    const int shift = qMin(m_reconnectAttempts, 5);
    const int delayMs = qMin(kBaseBackoffMs * (1 << shift), kMaxBackoffMs);
    ++m_reconnectAttempts;
    m_reconnectTimer->start(delayMs);
}

void IpcClient::onReconnectTimeout() {
    if (m_manualDisconnect || m_targetSocketPath.isEmpty()) {
        return;
    }
    m_readBuffer.clear();
    m_peerVerified = false;
    m_socket->abort();
    m_socket->connectToServer(m_targetSocketPath);
}

} // namespace botui
