#pragma once

#include <QAbstractListModel>
#include <QJsonArray>
#include <QJsonObject>
#include <QString>
#include <QVector>

namespace botui {

struct PositionItem {
    QString id;
    QString venue;
    QString symbol;
    QString strategy;
    QString state;
    QString entry;
    QString mark;
    QString qty;
    QString stop;
    QString tp1;
    QString tp2;
    QString profitR;
    QString unrealizedQuote;
    qint64 ageS = 0;
    bool tp1Done = false;
    bool beDone = false;
};

class PositionsModel : public QAbstractListModel {
    Q_OBJECT
    Q_PROPERTY(int count READ rowCount NOTIFY countChanged)

public:
    enum Roles {
        IdRole = Qt::UserRole + 1,
        VenueRole,
        SymbolRole,
        StrategyRole,
        StateRole,
        EntryRole,
        MarkRole,
        QtyRole,
        StopRole,
        Tp1Role,
        Tp2Role,
        ProfitRRole,
        UnrealizedQuoteRole,
        AgeSRole,
        Tp1DoneRole,
        BeDoneRole
    };

    explicit PositionsModel(QObject* parent = nullptr);

    int rowCount(const QModelIndex& parent = QModelIndex()) const override;
    QVariant data(const QModelIndex& index, int role = Qt::DisplayRole) const override;
    QHash<int, QByteArray> roleNames() const override;

    void replaceFromJson(const QJsonArray& arr);
    QVariantMap firstPositionForSymbol(const QString& symbol) const;
    QString firstSymbolOrDefault(const QString& fallback = QStringLiteral("BTCUSDT")) const;

signals:
    void countChanged();

private:
    QVector<PositionItem> m_items;
};

struct OrderItem {
    QString clientId;
    QString venue;
    QString symbol;
    QString side;
    QString kind;
    QString purpose;
    QString status;
    QString price;
    QString qty;
    QString filledQty;
};

class OrdersModel : public QAbstractListModel {
    Q_OBJECT
    Q_PROPERTY(int count READ rowCount NOTIFY countChanged)

public:
    enum Roles {
        ClientIdRole = Qt::UserRole + 1,
        VenueRole,
        SymbolRole,
        SideRole,
        KindRole,
        PurposeRole,
        StatusRole,
        PriceRole,
        QtyRole,
        FilledQtyRole
    };

    explicit OrdersModel(QObject* parent = nullptr);

    int rowCount(const QModelIndex& parent = QModelIndex()) const override;
    QVariant data(const QModelIndex& index, int role = Qt::DisplayRole) const override;
    QHash<int, QByteArray> roleNames() const override;

    void replaceFromJson(const QJsonArray& arr);

signals:
    void countChanged();

private:
    QVector<OrderItem> m_items;
};

struct SignalItem {
    QString venue;
    QString symbol;
    QString strategy;
    QString direction;
    QString entry;
    QString stop;
    qint64 tsMs = 0;
};

class SignalsModel : public QAbstractListModel {
    Q_OBJECT
    Q_PROPERTY(int count READ rowCount NOTIFY countChanged)

public:
    enum Roles {
        VenueRole = Qt::UserRole + 1,
        SymbolRole,
        StrategyRole,
        DirectionRole,
        EntryRole,
        StopRole,
        TsMsRole
    };

    explicit SignalsModel(QObject* parent = nullptr);

    int rowCount(const QModelIndex& parent = QModelIndex()) const override;
    QVariant data(const QModelIndex& index, int role = Qt::DisplayRole) const override;
    QHash<int, QByteArray> roleNames() const override;

    void replaceFromJson(const QJsonArray& arr);

signals:
    void countChanged();

private:
    QVector<SignalItem> m_items;
};

struct MarketTickItem {
    QString venue;
    QString symbol;
    QString bid;
    QString ask;
    QString last;
    QString spreadBps;
    qint64 tsMs = 0;
};

class MarketTicksModel : public QAbstractListModel {
    Q_OBJECT
    Q_PROPERTY(int count READ rowCount NOTIFY countChanged)

public:
    enum Roles {
        VenueRole = Qt::UserRole + 1,
        SymbolRole,
        BidRole,
        AskRole,
        LastRole,
        SpreadBpsRole,
        TsMsRole
    };

    explicit MarketTicksModel(QObject* parent = nullptr);

    int rowCount(const QModelIndex& parent = QModelIndex()) const override;
    QVariant data(const QModelIndex& index, int role = Qt::DisplayRole) const override;
    QHash<int, QByteArray> roleNames() const override;

    void replaceFromJson(const QJsonArray& arr);
    QStringList symbols() const;

signals:
    void countChanged();

private:
    QVector<MarketTickItem> m_items;
};

struct EventItem {
    quint64 seq = 0;
    qint64 tsMs = 0;
    QString venue;
    QString symbol;
    QString kind;
    QString level;
    QString message;
};

class EventsModel : public QAbstractListModel {
    Q_OBJECT
    Q_PROPERTY(int count READ rowCount NOTIFY countChanged)

public:
    enum Roles {
        SeqRole = Qt::UserRole + 1,
        TsMsRole,
        VenueRole,
        SymbolRole,
        KindRole,
        LevelRole,
        MessageRole
    };

    explicit EventsModel(QObject* parent = nullptr);

    int rowCount(const QModelIndex& parent = QModelIndex()) const override;
    QVariant data(const QModelIndex& index, int role = Qt::DisplayRole) const override;
    QHash<int, QByteArray> roleNames() const override;

    void prependEvent(quint64 seq, qint64 tsMs, const QJsonObject& payload, int maxRetained = 250);
    void clear();

signals:
    void countChanged();

private:
    QVector<EventItem> m_items;
};

struct AlertItem {
    QString id;
    QString venue;
    QString level;
    QString msg;
    bool acked = false;
};

class AlertsModel : public QAbstractListModel {
    Q_OBJECT
    Q_PROPERTY(int count READ rowCount NOTIFY countChanged)

public:
    enum Roles {
        IdRole = Qt::UserRole + 1,
        VenueRole,
        LevelRole,
        MsgRole,
        AckedRole
    };

    explicit AlertsModel(QObject* parent = nullptr);

    int rowCount(const QModelIndex& parent = QModelIndex()) const override;
    QVariant data(const QModelIndex& index, int role = Qt::DisplayRole) const override;
    QHash<int, QByteArray> roleNames() const override;

    void replaceFromJson(const QJsonArray& arr);
    int unackedCount() const;
    const QVector<AlertItem>& items() const { return m_items; }

signals:
    void countChanged();

private:
    QVector<AlertItem> m_items;
};

struct TradeItem {
    QString venue;
    QString symbol;
    QString side;
    QString price;
    QString qty;
    QString fee;
    QString realizedPnl;
    qint64 tsMs = 0;
};

class TradesModel : public QAbstractListModel {
    Q_OBJECT
    Q_PROPERTY(int count READ rowCount NOTIFY countChanged)

public:
    enum Roles {
        VenueRole = Qt::UserRole + 1,
        SymbolRole,
        SideRole,
        PriceRole,
        QtyRole,
        FeeRole,
        RealizedPnlRole,
        TsMsRole
    };

    explicit TradesModel(QObject* parent = nullptr);

    int rowCount(const QModelIndex& parent = QModelIndex()) const override;
    QVariant data(const QModelIndex& index, int role = Qt::DisplayRole) const override;
    QHash<int, QByteArray> roleNames() const override;

    void replaceFromJson(const QJsonArray& arr);

signals:
    void countChanged();

private:
    QVector<TradeItem> m_items;
};

} // namespace botui
