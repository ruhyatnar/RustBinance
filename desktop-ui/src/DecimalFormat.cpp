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
