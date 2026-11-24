#pragma once

#include <QObject>
#include <QString>

namespace botui {

/**
 * Pure string-based decimal formatter and sign inspector (Hard Rule #4, §11.1, §11.5).
 *
 * Monetary/decimal strings from IPC v1 are formatted via exact character manipulation
 * without converting to IEEE-754 floating-point, preserving arbitrary precision.
 */
class DecimalFormat : public QObject {
    Q_OBJECT

public:
    explicit DecimalFormat(QObject* parent = nullptr);

    Q_INVOKABLE QString formatDecimal(const QString& raw, int maxFrac = -1) const;
    Q_INVOKABLE int decimalSign(const QString& raw) const;
    Q_INVOKABLE QString directionGlyph(const QString& raw) const;

    /**
     * Convert decimal string to double strictly for Canvas pixel coordinates
     * and RiskGauge bar ratios (never for displayed text).
     */
    Q_INVOKABLE double toPlotNumber(const QString& raw) const;
};

} // namespace botui
