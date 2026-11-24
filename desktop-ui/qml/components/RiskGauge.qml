import QtQuick
import QtQuick.Layouts
import BinanceBot

PanelCard {
    id: gauge
    title: "RISK & EXPOSURE"
    subtitle: store ? ("BTC Guard: " + store.btcGuard) : ""

    property var store
    property var fmt

    function clampRatio(num, den) {
        var d = Math.abs(den)
        if (d <= 0) return 0.0
        var r = Math.abs(num) / d
        return Math.max(0.0, Math.min(1.0, r))
    }

    // Portfolio Heat bar
    ColumnLayout {
        Layout.fillWidth: true
        spacing: 4

        RowLayout {
            Layout.fillWidth: true
            Text {
                text: "Portfolio Heat"
                color: Theme.textSecondary
                font.pixelSize: 12
                Layout.fillWidth: true
            }
            Text {
                color: Theme.textPrimary
                font.pixelSize: 12
                font.family: Theme.monoFont
                text: (gauge.store && gauge.fmt)
                      ? (gauge.fmt.formatDecimal(gauge.store.heatPct, 2) + "% / " + gauge.fmt.formatDecimal(gauge.store.heatMaxPct, 2) + "%")
                      : "—"
            }
        }

        Rectangle {
            Layout.fillWidth: true
            height: 8
            radius: 4
            color: Theme.surfaceAltBg

            Rectangle {
                height: parent.height
                radius: 4
                width: {
                    if (!gauge.store || !gauge.fmt) return 0
                    var ratio = gauge.clampRatio(
                        gauge.fmt.toPlotNumber(gauge.store.heatPct),
                        gauge.fmt.toPlotNumber(gauge.store.heatMaxPct)
                    )
                    return parent.width * ratio
                }
                color: Theme.accent
            }
        }
    }

    // Daily Loss Halt proximity bar
    ColumnLayout {
        Layout.fillWidth: true
        spacing: 4

        RowLayout {
            Layout.fillWidth: true
            Text {
                text: "Day PnL vs Halt"
                color: Theme.textSecondary
                font.pixelSize: 12
                Layout.fillWidth: true
            }
            Text {
                font.pixelSize: 12
                font.family: Theme.monoFont
                color: (gauge.store && gauge.fmt)
                       ? Theme.signColor(gauge.fmt.decimalSign(gauge.store.dayPnlPct))
                       : Theme.textPrimary
                text: (gauge.store && gauge.fmt)
                      ? (gauge.fmt.directionGlyph(gauge.store.dayPnlPct) + " "
                         + gauge.fmt.formatDecimal(gauge.store.dayPnlPct, 2) + "% (halt "
                         + gauge.fmt.formatDecimal(gauge.store.dailyHaltPct, 2) + "%)")
                      : "—"
            }
        }

        Rectangle {
            Layout.fillWidth: true
            height: 8
            radius: 4
            color: Theme.surfaceAltBg

            Rectangle {
                height: parent.height
                radius: 4
                width: {
                    if (!gauge.store || !gauge.fmt) return 0
                    var dayVal = gauge.fmt.toPlotNumber(gauge.store.dayPnlPct)
                    if (dayVal >= 0) return 0
                    var ratio = gauge.clampRatio(dayVal, gauge.fmt.toPlotNumber(gauge.store.dailyHaltPct))
                    return parent.width * ratio
                }
                color: Theme.warning
            }
        }
    }

    // Drawdown vs Kill bar
    ColumnLayout {
        Layout.fillWidth: true
        spacing: 4

        RowLayout {
            Layout.fillWidth: true
            Text {
                text: "Drawdown vs Kill"
                color: Theme.textSecondary
                font.pixelSize: 12
                Layout.fillWidth: true
            }
            Text {
                font.pixelSize: 12
                font.family: Theme.monoFont
                color: Theme.negative
                text: (gauge.store && gauge.fmt)
                      ? (gauge.fmt.formatDecimal(gauge.store.drawdownPct, 2) + "% / "
                         + gauge.fmt.formatDecimal(gauge.store.killPct, 2) + "%")
                      : "—"
            }
        }

        Rectangle {
            Layout.fillWidth: true
            height: 8
            radius: 4
            color: Theme.surfaceAltBg

            Rectangle {
                height: parent.height
                radius: 4
                width: {
                    if (!gauge.store || !gauge.fmt) return 0
                    var ratio = gauge.clampRatio(
                        gauge.fmt.toPlotNumber(gauge.store.drawdownPct),
                        gauge.fmt.toPlotNumber(gauge.store.killPct)
                    )
                    return parent.width * ratio
                }
                color: Theme.negative
            }
        }
    }

    RowLayout {
        Layout.fillWidth: true
        Text {
            text: "Open Positions"
            color: Theme.textSecondary
            font.pixelSize: 12
            Layout.fillWidth: true
        }
        Text {
            color: Theme.textPrimary
            font.pixelSize: 12
            font.family: Theme.monoFont
            text: gauge.store ? (gauge.store.openPositions + " / " + gauge.store.maxPositions) : "—"
        }
    }
}
