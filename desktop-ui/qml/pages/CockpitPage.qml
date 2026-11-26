import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import BinanceBot

Item {
    id: cockpit

    property var store
    property var commands
    property var fmt

    readonly property var timeframes: ["1m", "5m", "15m", "1h", "4h"]

    ScrollView {
        anchors.fill: parent
        contentWidth: availableWidth

        ColumnLayout {
            width: parent.width
            spacing: 12

            // Operator entry controls (visible only when unlocked)
            RowLayout {
                Layout.fillWidth: true
                visible: cockpit.store ? cockpit.store.canMutate : false
                spacing: 8

                Button {
                    text: "❚❚ Pause Entries"
                    enabled: cockpit.commands ? !cockpit.commands.busy : false
                    onClicked: if (cockpit.commands) cockpit.commands.pause()
                }

                Button {
                    text: "▶ Resume Entries"
                    enabled: cockpit.commands ? !cockpit.commands.busy : false
                    onClicked: if (cockpit.commands) cockpit.commands.resume()
                }

                Item { Layout.fillWidth: true }
            }

            RowLayout {
                Layout.fillWidth: true
                spacing: 12

                // Left column: Risk + Shadow Signals
                ColumnLayout {
                    Layout.preferredWidth: 340
                    Layout.fillHeight: true
                    spacing: 12

                    RiskGauge {
                        Layout.fillWidth: true
                        store: cockpit.store
                        fmt: cockpit.fmt
                    }

                    PanelCard {
                        Layout.fillWidth: true
                        title: "RECENT SIGNALS"
                        subtitle: (cockpit.store && cockpit.store.signalsModel)
                                  ? (cockpit.store.signalsModel.count + " shadow")
                                  : ""

                        ListView {
                            id: sigList
                            Layout.fillWidth: true
                            Layout.preferredHeight: Math.max(96, contentHeight)
                            clip: true
                            model: cockpit.store ? cockpit.store.signalsModel : null

                            delegate: RowLayout {
                                width: sigList.width
                                height: 26
                                spacing: 6

                                Text {
                                    text: model.venue.toUpperCase()
                                    color: Theme.accent
                                    font.pixelSize: 10
                                    font.bold: true
                                }
                                Text {
                                    readonly property bool isShort: model.direction.toUpperCase() === "SHORT"
                                    text: (isShort ? "▼ " : "▲ ") + model.symbol
                                    color: isShort ? Theme.negative : Theme.positive
                                    font.pixelSize: 12
                                    font.bold: true
                                }
                                Text {
                                    text: model.strategy
                                    color: Theme.textSecondary
                                    font.pixelSize: 11
                                    Layout.fillWidth: true
                                    elide: Text.ElideRight
                                }
                                Text {
                                    text: cockpit.fmt ? cockpit.fmt.formatPrice(model.entry) : model.entry
                                    color: Theme.textPrimary
                                    font.pixelSize: 11
                                    font.family: Theme.monoFont
                                }
                            }
                        }

                        Text {
                            visible: !cockpit.store || !cockpit.store.signalsModel || cockpit.store.signalsModel.count === 0
                            text: "No shadow signals in current snapshot."
                            color: Theme.textSecondary
                            font.pixelSize: 12
                        }
                    }
                }

                // Right column: Open Positions + Candlestick Chart
                ColumnLayout {
                    Layout.fillWidth: true
                    Layout.fillHeight: true
                    spacing: 12

                    PanelCard {
                        Layout.fillWidth: true
                        title: "OPEN POSITIONS"
                        subtitle: (cockpit.store && cockpit.store.positionsModel)
                                  ? (cockpit.store.positionsModel.count + " open")
                                  : ""

                        ListView {
                            id: posList
                            Layout.fillWidth: true
                            Layout.preferredHeight: Math.max(88, contentHeight)
                            clip: true
                            model: cockpit.store ? cockpit.store.positionsModel : null

                            delegate: RowLayout {
                                width: posList.width
                                height: 32
                                spacing: 8

                                Text {
                                    text: model.venue.toUpperCase()
                                    color: Theme.accent
                                    font.pixelSize: 11
                                    font.bold: true
                                    Layout.preferredWidth: 48
                                }
                                Text {
                                    text: model.symbol
                                    color: Theme.textPrimary
                                    font.pixelSize: 12
                                    font.bold: true
                                    Layout.preferredWidth: 84
                                }
                                Text {
                                    readonly property int sgn: cockpit.fmt ? cockpit.fmt.decimalSign(model.profitR) : 0
                                    text: (cockpit.fmt ? cockpit.fmt.directionGlyph(model.profitR) : "") + " "
                                          + (cockpit.fmt ? cockpit.fmt.formatDecimal(model.profitR, 2) : model.profitR) + "R"
                                    color: Theme.signColor(sgn)
                                    font.pixelSize: 12
                                    font.family: Theme.monoFont
                                    Layout.preferredWidth: 78
                                }
                                Text {
                                    readonly property int qSgn: cockpit.fmt ? cockpit.fmt.decimalSign(model.unrealizedQuote) : 0
                                    text: (cockpit.fmt ? cockpit.fmt.formatDecimal(model.unrealizedQuote, 2) : model.unrealizedQuote)
                                    color: Theme.signColor(qSgn)
                                    font.pixelSize: 12
                                    font.family: Theme.monoFont
                                    Layout.preferredWidth: 84
                                }
                                Text {
                                    text: "SL " + (cockpit.fmt ? cockpit.fmt.formatPrice(model.stop) : model.stop)
                                          + (model.beDone ? " (BE)" : "")
                                          + " · TP1 " + (cockpit.fmt ? cockpit.fmt.formatPrice(model.tp1) : model.tp1)
                                    color: Theme.textSecondary
                                    font.pixelSize: 11
                                    font.family: Theme.monoFont
                                    Layout.fillWidth: true
                                    elide: Text.ElideRight
                                }
                                Button {
                                    visible: cockpit.store ? cockpit.store.canMutate : false
                                    text: "Close"
                                    onClicked: if (cockpit.commands) cockpit.commands.closePosition(model.positionId)
                                }
                            }
                        }

                        Text {
                            visible: !cockpit.store || !cockpit.store.positionsModel || cockpit.store.positionsModel.count === 0
                            text: "No open positions."
                            color: Theme.textSecondary
                            font.pixelSize: 12
                        }
                    }

                    RowLayout {
                        Layout.fillWidth: true
                        spacing: 6

                        Repeater {
                            model: cockpit.timeframes
                            delegate: Button {
                                text: modelData
                                highlighted: cockpit.store ? (cockpit.store.chartTf === modelData) : false
                                onClicked: {
                                    if (cockpit.store) {
                                        cockpit.store.chartTf = modelData
                                        if (cockpit.commands) {
                                            cockpit.commands.refreshKlines(
                                                cockpit.store.activeVenue,
                                                cockpit.store.chartSymbol,
                                                modelData,
                                                120)
                                        }
                                    }
                                }
                            }
                        }

                        Item { Layout.fillWidth: true }

                        Button {
                            text: "↻ Refresh Chart"
                            onClicked: {
                                if (cockpit.store && cockpit.commands) {
                                    cockpit.commands.refreshKlines(
                                        cockpit.store.activeVenue,
                                        cockpit.store.chartSymbol,
                                        cockpit.store.chartTf,
                                        120)
                                }
                            }
                        }
                    }

                    CandlesChart {
                        Layout.fillWidth: true
                        symbol: cockpit.store ? cockpit.store.chartSymbol : "BTCUSDT"
                        timeframe: cockpit.store ? cockpit.store.chartTf : "15m"
                        bars: cockpit.store ? cockpit.store.klines : []
                        positionOverlay: cockpit.store ? cockpit.store.activeChartPosition : ({})
                        fmt: cockpit.fmt
                        chartHeight: 240
                    }
                }
            }

            EventList {
                Layout.fillWidth: true
                eventsModel: cockpit.store ? cockpit.store.eventsModel : null
                preferredListHeight: 150
            }
        }
    }
}
