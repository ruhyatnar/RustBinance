#include "Models.h"

namespace botui {

PositionsModel::PositionsModel(QObject* parent) : QAbstractListModel(parent) {}

int PositionsModel::rowCount(const QModelIndex& parent) const {
    return parent.isValid() ? 0 : m_items.size();
}

QVariant PositionsModel::data(const QModelIndex& index, int role) const {
    if (!index.isValid() || index.row() < 0 || index.row() >= m_items.size()) {
        return {};
    }
    const PositionItem& item = m_items.at(index.row());
    switch (role) {
        case IdRole: return item.id;
        case VenueRole: return item.venue;
        case SymbolRole: return item.symbol;
        case StrategyRole: return item.strategy;
        case StateRole: return item.state;
        case EntryRole: return item.entry;
        case MarkRole: return item.mark;
        case QtyRole: return item.qty;
        case StopRole: return item.stop;
        case Tp1Role: return item.tp1;
        case Tp2Role: return item.tp2;
        case ProfitRRole: return item.profitR;
        case UnrealizedQuoteRole: return item.unrealizedQuote;
        case AgeSRole: return item.ageS;
        case Tp1DoneRole: return item.tp1Done;
        case BeDoneRole: return item.beDone;
        default: return {};
    }
}

QHash<int, QByteArray> PositionsModel::roleNames() const {
    return {
        {IdRole, "positionId"},
        {VenueRole, "venue"},
        {SymbolRole, "symbol"},
        {StrategyRole, "strategy"},
        {StateRole, "state"},
        {EntryRole, "entry"},
        {MarkRole, "mark"},
        {QtyRole, "qty"},
        {StopRole, "stop"},
        {Tp1Role, "tp1"},
        {Tp2Role, "tp2"},
        {ProfitRRole, "profitR"},
        {UnrealizedQuoteRole, "unrealizedQuote"},
        {AgeSRole, "ageS"},
        {Tp1DoneRole, "tp1Done"},
        {BeDoneRole, "beDone"},
    };
}

void PositionsModel::replaceFromJson(const QJsonArray& arr) {
    beginResetModel();
    m_items.clear();
    m_items.reserve(arr.size());
    for (const QJsonValue& val : arr) {
        const QJsonObject obj = val.toObject();
        PositionItem item;
        item.id = obj.value(QStringLiteral("id")).toString();
        item.venue = obj.value(QStringLiteral("venue")).toString();
        item.symbol = obj.value(QStringLiteral("symbol")).toString();
        item.strategy = obj.value(QStringLiteral("strategy")).toString();
        item.state = obj.value(QStringLiteral("state")).toString();
        item.entry = obj.value(QStringLiteral("entry")).toString();
        item.mark = obj.value(QStringLiteral("mark")).toString();
        item.qty = obj.value(QStringLiteral("qty")).toString();
        item.stop = obj.value(QStringLiteral("stop")).toString();
        item.tp1 = obj.value(QStringLiteral("tp1")).toString();
        item.tp2 = obj.value(QStringLiteral("tp2")).toString();
        item.profitR = obj.value(QStringLiteral("profit_r")).toString();
        item.unrealizedQuote = obj.value(QStringLiteral("unrealized_quote")).toString();
        item.ageS = obj.value(QStringLiteral("age_s")).toInteger();
        item.tp1Done = obj.value(QStringLiteral("tp1_done")).toBool();
        item.beDone = obj.value(QStringLiteral("be_done")).toBool();
        m_items.push_back(item);
    }
    endResetModel();
    emit countChanged();
}

QVariantMap PositionsModel::firstPositionForSymbol(const QString& symbol) const {
    for (const PositionItem& item : m_items) {
        if (item.symbol == symbol) {
            return {
                {QStringLiteral("entry"), item.entry},
                {QStringLiteral("stop"), item.stop},
                {QStringLiteral("tp1"), item.tp1},
                {QStringLiteral("tp2"), item.tp2},
                {QStringLiteral("venue"), item.venue},
            };
        }
    }
    return {};
}

QString PositionsModel::firstSymbolOrDefault(const QString& fallback) const {
    return m_items.isEmpty() ? fallback : m_items.first().symbol;
}

OrdersModel::OrdersModel(QObject* parent) : QAbstractListModel(parent) {}

int OrdersModel::rowCount(const QModelIndex& parent) const {
    return parent.isValid() ? 0 : m_items.size();
}

QVariant OrdersModel::data(const QModelIndex& index, int role) const {
    if (!index.isValid() || index.row() < 0 || index.row() >= m_items.size()) {
        return {};
    }
    const OrderItem& item = m_items.at(index.row());
    switch (role) {
        case ClientIdRole: return item.clientId;
        case VenueRole: return item.venue;
        case SymbolRole: return item.symbol;
        case SideRole: return item.side;
        case KindRole: return item.kind;
        case PurposeRole: return item.purpose;
        case StatusRole: return item.status;
        case PriceRole: return item.price;
        case QtyRole: return item.qty;
        case FilledQtyRole: return item.filledQty;
        default: return {};
    }
}

QHash<int, QByteArray> OrdersModel::roleNames() const {
    return {
        {ClientIdRole, "clientId"},
        {VenueRole, "venue"},
        {SymbolRole, "symbol"},
        {SideRole, "side"},
        {KindRole, "kind"},
        {PurposeRole, "purpose"},
        {StatusRole, "status"},
        {PriceRole, "price"},
        {QtyRole, "qty"},
        {FilledQtyRole, "filledQty"},
    };
}

void OrdersModel::replaceFromJson(const QJsonArray& arr) {
    beginResetModel();
    m_items.clear();
    m_items.reserve(arr.size());
    for (const QJsonValue& val : arr) {
        const QJsonObject obj = val.toObject();
        OrderItem item;
        item.clientId = obj.value(QStringLiteral("client_id")).toString();
        item.venue = obj.value(QStringLiteral("venue")).toString();
        item.symbol = obj.value(QStringLiteral("symbol")).toString();
        item.side = obj.value(QStringLiteral("side")).toString();
        item.kind = obj.value(QStringLiteral("kind")).toString();
        item.purpose = obj.value(QStringLiteral("purpose")).toString();
        item.status = obj.value(QStringLiteral("status")).toString();
        item.price = obj.value(QStringLiteral("price")).isNull()
                         ? QString()
                         : obj.value(QStringLiteral("price")).toString();
        item.qty = obj.value(QStringLiteral("qty")).toString();
        item.filledQty = obj.value(QStringLiteral("filled_qty")).toString();
        m_items.push_back(item);
    }
    endResetModel();
    emit countChanged();
}

SignalsModel::SignalsModel(QObject* parent) : QAbstractListModel(parent) {}

int SignalsModel::rowCount(const QModelIndex& parent) const {
    return parent.isValid() ? 0 : m_items.size();
}

QVariant SignalsModel::data(const QModelIndex& index, int role) const {
    if (!index.isValid() || index.row() < 0 || index.row() >= m_items.size()) {
        return {};
    }
    const SignalItem& item = m_items.at(index.row());
    switch (role) {
        case VenueRole: return item.venue;
        case SymbolRole: return item.symbol;
        case StrategyRole: return item.strategy;
        case DirectionRole: return item.direction;
        case EntryRole: return item.entry;
        case StopRole: return item.stop;
        case TsMsRole: return item.tsMs;
        default: return {};
    }
}

QHash<int, QByteArray> SignalsModel::roleNames() const {
    return {
        {VenueRole, "venue"},
        {SymbolRole, "symbol"},
        {StrategyRole, "strategy"},
        {DirectionRole, "direction"},
        {EntryRole, "entry"},
        {StopRole, "stop"},
        {TsMsRole, "tsMs"},
    };
}

void SignalsModel::replaceFromJson(const QJsonArray& arr) {
    beginResetModel();
    m_items.clear();
    m_items.reserve(arr.size());
    for (const QJsonValue& val : arr) {
        const QJsonObject obj = val.toObject();
        SignalItem item;
        item.venue = obj.value(QStringLiteral("venue")).toString();
        item.symbol = obj.value(QStringLiteral("symbol")).toString();
        item.strategy = obj.value(QStringLiteral("strategy")).toString();
        item.direction = obj.value(QStringLiteral("direction")).toString();
        item.entry = obj.value(QStringLiteral("entry")).toString();
        item.stop = obj.value(QStringLiteral("stop")).toString();
        item.tsMs = obj.value(QStringLiteral("ts_ms")).toInteger();
        m_items.push_back(item);
    }
    endResetModel();
    emit countChanged();
}

MarketTicksModel::MarketTicksModel(QObject* parent) : QAbstractListModel(parent) {}

int MarketTicksModel::rowCount(const QModelIndex& parent) const {
    return parent.isValid() ? 0 : m_items.size();
}

QVariant MarketTicksModel::data(const QModelIndex& index, int role) const {
    if (!index.isValid() || index.row() < 0 || index.row() >= m_items.size()) {
        return {};
    }
    const MarketTickItem& item = m_items.at(index.row());
    switch (role) {
        case VenueRole: return item.venue;
        case SymbolRole: return item.symbol;
        case BidRole: return item.bid;
        case AskRole: return item.ask;
        case LastRole: return item.last;
        case SpreadBpsRole: return item.spreadBps;
        case TsMsRole: return item.tsMs;
        default: return {};
    }
}

QHash<int, QByteArray> MarketTicksModel::roleNames() const {
    return {
        {VenueRole, "venue"},
        {SymbolRole, "symbol"},
        {BidRole, "bid"},
        {AskRole, "ask"},
        {LastRole, "last"},
        {SpreadBpsRole, "spreadBps"},
        {TsMsRole, "tsMs"},
    };
}

void MarketTicksModel::replaceFromJson(const QJsonArray& arr) {
    beginResetModel();
    m_items.clear();
    m_items.reserve(arr.size());
    for (const QJsonValue& val : arr) {
        const QJsonObject obj = val.toObject();
        MarketTickItem item;
        item.venue = obj.value(QStringLiteral("venue")).toString();
        item.symbol = obj.value(QStringLiteral("symbol")).toString();
        item.bid = obj.value(QStringLiteral("bid")).toString();
        item.ask = obj.value(QStringLiteral("ask")).toString();
        item.last = obj.value(QStringLiteral("last")).toString();
        item.spreadBps = obj.value(QStringLiteral("spread_bps")).toString();
        item.tsMs = obj.value(QStringLiteral("ts_ms")).toInteger();
        m_items.push_back(item);
    }
    endResetModel();
    emit countChanged();
}

QStringList MarketTicksModel::symbols() const {
    QStringList out;
    for (const MarketTickItem& item : m_items) {
        if (!item.symbol.isEmpty() && !out.contains(item.symbol)) {
            out.push_back(item.symbol);
        }
    }
    return out;
}

EventsModel::EventsModel(QObject* parent) : QAbstractListModel(parent) {}

int EventsModel::rowCount(const QModelIndex& parent) const {
    return parent.isValid() ? 0 : m_items.size();
}

QVariant EventsModel::data(const QModelIndex& index, int role) const {
    if (!index.isValid() || index.row() < 0 || index.row() >= m_items.size()) {
        return {};
    }
    const EventItem& item = m_items.at(index.row());
    switch (role) {
        case SeqRole: return static_cast<qulonglong>(item.seq);
        case TsMsRole: return item.tsMs;
        case VenueRole: return item.venue;
        case SymbolRole: return item.symbol;
        case KindRole: return item.kind;
        case LevelRole: return item.level;
        case MessageRole: return item.message;
        default: return {};
    }
}

QHash<int, QByteArray> EventsModel::roleNames() const {
    return {
        {SeqRole, "seq"},
        {TsMsRole, "tsMs"},
        {VenueRole, "venue"},
        {SymbolRole, "symbol"},
        {KindRole, "kind"},
        {LevelRole, "level"},
        {MessageRole, "message"},
    };
}

void EventsModel::prependEvent(quint64 seq, qint64 tsMs, const QJsonObject& payload, int maxRetained) {
    EventItem item;
    item.seq = seq;
    item.tsMs = tsMs;
    item.venue = payload.value(QStringLiteral("venue")).toString();
    item.symbol = payload.value(QStringLiteral("symbol")).toString();
    item.kind = payload.value(QStringLiteral("kind")).toString();
    item.level = payload.value(QStringLiteral("level")).toString();
    item.message = payload.value(QStringLiteral("message")).toString();

    beginInsertRows(QModelIndex(), 0, 0);
    m_items.prepend(item);
    endInsertRows();

    if (m_items.size() > maxRetained) {
        beginRemoveRows(QModelIndex(), maxRetained, m_items.size() - 1);
        m_items.resize(maxRetained);
        endRemoveRows();
    }
    emit countChanged();
}

void EventsModel::clear() {
    if (m_items.isEmpty()) {
        return;
    }
    beginResetModel();
    m_items.clear();
    endResetModel();
    emit countChanged();
}

AlertsModel::AlertsModel(QObject* parent) : QAbstractListModel(parent) {}

int AlertsModel::rowCount(const QModelIndex& parent) const {
    return parent.isValid() ? 0 : m_items.size();
}

QVariant AlertsModel::data(const QModelIndex& index, int role) const {
    if (!index.isValid() || index.row() < 0 || index.row() >= m_items.size()) {
        return {};
    }
    const AlertItem& item = m_items.at(index.row());
    switch (role) {
        case IdRole: return item.id;
        case VenueRole: return item.venue;
        case LevelRole: return item.level;
        case MsgRole: return item.msg;
        case AckedRole: return item.acked;
        default: return {};
    }
}

QHash<int, QByteArray> AlertsModel::roleNames() const {
    return {
        {IdRole, "alertId"},
        {VenueRole, "venue"},
        {LevelRole, "level"},
        {MsgRole, "msg"},
        {AckedRole, "acked"},
    };
}

void AlertsModel::replaceFromJson(const QJsonArray& arr) {
    beginResetModel();
    m_items.clear();
    m_items.reserve(arr.size());
    for (const QJsonValue& val : arr) {
        const QJsonObject obj = val.toObject();
        AlertItem item;
        item.id = obj.value(QStringLiteral("id")).toString();
        item.venue = obj.value(QStringLiteral("venue")).toString();
        item.level = obj.value(QStringLiteral("level")).toString();
        item.msg = obj.value(QStringLiteral("msg")).toString();
        item.acked = obj.value(QStringLiteral("acked")).toBool(false);
        m_items.push_back(item);
    }
    endResetModel();
    emit countChanged();
}

int AlertsModel::unackedCount() const {
    int count = 0;
    for (const AlertItem& item : m_items) {
        if (!item.acked) {
            ++count;
        }
    }
    return count;
}

TradesModel::TradesModel(QObject* parent) : QAbstractListModel(parent) {}

int TradesModel::rowCount(const QModelIndex& parent) const {
    return parent.isValid() ? 0 : m_items.size();
}

QVariant TradesModel::data(const QModelIndex& index, int role) const {
    if (!index.isValid() || index.row() < 0 || index.row() >= m_items.size()) {
        return {};
    }
    const TradeItem& item = m_items.at(index.row());
    switch (role) {
        case VenueRole: return item.venue;
        case SymbolRole: return item.symbol;
        case SideRole: return item.side;
        case PriceRole: return item.price;
        case QtyRole: return item.qty;
        case FeeRole: return item.fee;
        case RealizedPnlRole: return item.realizedPnl;
        case TsMsRole: return item.tsMs;
        default: return {};
    }
}

QHash<int, QByteArray> TradesModel::roleNames() const {
    return {
        {VenueRole, "venue"},
        {SymbolRole, "symbol"},
        {SideRole, "side"},
        {PriceRole, "price"},
        {QtyRole, "qty"},
        {FeeRole, "fee"},
        {RealizedPnlRole, "realizedPnl"},
        {TsMsRole, "tsMs"},
    };
}

void TradesModel::replaceFromJson(const QJsonArray& arr) {
    beginResetModel();
    m_items.clear();
    m_items.reserve(arr.size());
    for (const QJsonValue& val : arr) {
        const QJsonObject obj = val.toObject();
        TradeItem item;
        item.venue = obj.value(QStringLiteral("venue")).toString();
        item.symbol = obj.value(QStringLiteral("symbol")).toString();
        item.side = obj.value(QStringLiteral("side")).toString();
        item.price = obj.value(QStringLiteral("price")).toString();
        item.qty = obj.value(QStringLiteral("qty")).toString();
        item.fee = obj.value(QStringLiteral("fee")).toString();
        item.realizedPnl = obj.value(QStringLiteral("realized_pnl")).toString();
        item.tsMs = obj.value(QStringLiteral("ts_ms")).toInteger();
        m_items.push_back(item);
    }
    endResetModel();
    emit countChanged();
}

} // namespace botui
