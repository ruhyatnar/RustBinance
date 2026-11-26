#include "DecimalFormat.h"

namespace botui {

namespace {

bool parseDecimalParts(const QString& raw, bool& negative, QString& intPart, QString& fracPart) {
    const QString trimmed = raw.trimmed();
    if (trimmed.isEmpty()) {
        return false;
    }

    int idx = 0;
    negative = false;
    if (trimmed.at(idx) == QLatin1Char('-')) {
        negative = true;
        ++idx;
    } else if (trimmed.at(idx) == QLatin1Char('+')) {
        ++idx;
    }

    if (idx >= trimmed.size()) {
        return false;
    }

    const int dotPos = trimmed.indexOf(QLatin1Char('.'), idx);
    if (dotPos == -1) {
        intPart = trimmed.mid(idx);
        fracPart.clear();
    } else {
        intPart = trimmed.mid(idx, dotPos - idx);
        fracPart = trimmed.mid(dotPos + 1);
        if (fracPart.isEmpty()) {
            return false;
        }
    }

    if (intPart.isEmpty()) {
        return false;
    }

    for (const QChar ch : intPart) {
        if (!ch.isDigit()) {
            return false;
        }
    }
    for (const QChar ch : fracPart) {
        if (!ch.isDigit()) {
            return false;
        }
    }
    return true;
}

bool allZeros(const QString& digits) {
    for (const QChar ch : digits) {
        if (ch != QLatin1Char('0')) {
            return false;
        }
    }
    return true;
}

QString groupThousands(const QString& digits) {
    QString normalized = digits;
    int firstNonZero = 0;
    while (firstNonZero + 1 < normalized.size() && normalized.at(firstNonZero) == QLatin1Char('0')) {
        ++firstNonZero;
    }
    normalized = normalized.mid(firstNonZero);

    QString out;
    out.reserve(normalized.size() + normalized.size() / 3);
    const int n = normalized.size();
    for (int i = 0; i < n; ++i) {
        if (i > 0 && ((n - i) % 3 == 0)) {
            out.append(QLatin1Char(','));
        }
        out.append(normalized.at(i));
    }
    return out;
}

} // namespace

DecimalFormat::DecimalFormat(QObject* parent) : QObject(parent) {}

QString DecimalFormat::formatDecimal(const QString& raw, int maxFrac) const {
    bool negative = false;
    QString intPart;
    QString fracPart;
    if (!parseDecimalParts(raw, negative, intPart, fracPart)) {
        return QStringLiteral("—");
    }

    if (maxFrac == 0) {
        fracPart.clear();
    } else if (maxFrac > 0 && fracPart.size() > maxFrac) {
        fracPart = fracPart.left(maxFrac);
    }

    const bool isZero = allZeros(intPart) && allZeros(fracPart);
    QString result;
    if (negative && !isZero) {
        result.append(QLatin1Char('-'));
    }
    result.append(groupThousands(intPart));
    if (!fracPart.isEmpty()) {
        result.append(QLatin1Char('.'));
        result.append(fracPart);
    }
    return result;
}

QString DecimalFormat::formatPrice(const QString& raw) const {
    bool negative = false;
    QString intPart;
    QString fracPart;
    if (!parseDecimalParts(raw, negative, intPart, fracPart)) {
        return QStringLiteral("—");
    }

    int firstIntNonZero = 0;
    while (firstIntNonZero + 1 < intPart.size() && intPart.at(firstIntNonZero) == QLatin1Char('0')) {
        ++firstIntNonZero;
    }
    const QString normInt = intPart.mid(firstIntNonZero);

    if (normInt != QStringLiteral("0")) {
        if (normInt.size() >= 2) {
            // Prices >= 10: standard 2 decimal places (e.g. BTC 68,120.50, SOL 142.35)
            return formatDecimal(raw, 2);
        }
        // Prices in [1, 10): keep up to 4 decimals, trim trailing zeros down to min 2
        if (fracPart.size() > 4) {
            fracPart = fracPart.left(4);
        }
        while (fracPart.size() > 2 && fracPart.endsWith(QLatin1Char('0'))) {
            fracPart.chop(1);
        }
    } else if (!allZeros(fracPart)) {
        // Sub-dollar / sub-cent prices (< 1): preserve 4 significant digits after leading zeros
        int leadingZeros = 0;
        while (leadingZeros < fracPart.size() && fracPart.at(leadingZeros) == QLatin1Char('0')) {
            ++leadingZeros;
        }
        const int keepLen = leadingZeros + 4;
        if (fracPart.size() > keepLen) {
            fracPart = fracPart.left(keepLen);
        }
        const int minFrac = (leadingZeros == 0) ? 2 : (leadingZeros + 1);
        while (fracPart.size() > minFrac && fracPart.endsWith(QLatin1Char('0'))) {
            fracPart.chop(1);
        }
    } else if (fracPart.size() > 2) {
        fracPart = fracPart.left(2);
    }

    const bool isZero = allZeros(normInt) && allZeros(fracPart);
    QString result;
    if (negative && !isZero) {
        result.append(QLatin1Char('-'));
    }
    result.append(groupThousands(normInt));
    if (!fracPart.isEmpty()) {
        result.append(QLatin1Char('.'));
        result.append(fracPart);
    }
    return result;
}

int DecimalFormat::decimalSign(const QString& raw) const {
    bool negative = false;
    QString intPart;
    QString fracPart;
    if (!parseDecimalParts(raw, negative, intPart, fracPart)) {
        return 0;
    }
    if (allZeros(intPart) && allZeros(fracPart)) {
        return 0;
    }
    return negative ? -1 : 1;
}

QString DecimalFormat::directionGlyph(const QString& raw) const {
    const int sign = decimalSign(raw);
    if (sign > 0) {
        return QStringLiteral("▲");
    }
    if (sign < 0) {
        return QStringLiteral("▼");
    }
    return QStringLiteral("•");
}

double DecimalFormat::toPlotNumber(const QString& raw) const {
    bool ok = false;
    const double val = raw.trimmed().toDouble(&ok);
    return ok ? val : 0.0;
}

} // namespace botui
