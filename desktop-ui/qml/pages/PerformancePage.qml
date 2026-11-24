import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import BinanceBot

Item {
    id: perfPage

    property var store
    property var commands
    property var fmt

    ColumnLayout {
        anchors.fill: parent
        spacing: 12

        RowLayout {
            Layout.fillWidth: true
            spacing: 12

            PanelCard {
                Layout.fillWidth: true
                title: "EQUITY"
                Text {
                    text: (perfPage.store && perfPage.fmt)
                          ? perfPage.fmt.formatDecimal(perfPage.store.equity, 2)
                          : "—"
                    color: Theme.textPrimary
                    font.pixelSize: 18
                    font.bold: true
                    font.family: Theme.monoFont
                }
            }

            PanelCard {
                Layout.fillWidth: true
                title: "DAY PNL"
                Text {
                    readonly property int sgn: (perfPage.store && perfPage.fmt)
                                               ? perfPage.fmt.decimalSign(perfPage.store.dayPnlPct)
                                               : 0
                    text: (perfPage.store && perfPage.fmt)
                          ? (perfPage.fmt.directionGlyph(perfPage.store.dayPnlPct) + " "
                             + perfPage.fmt.formatDecimal(perfPage.store.dayPnlPct, 2) + "%")
                          : "—"
                    color: Theme.signColor(sgn)
                    font.pixelSize: 18
                    font.bold: true
                    font.family: Theme.monoFont
                }
            }

            PanelCard {
                Layout.fillWidth: true
                title: "WEEK PNL"
                Text {
                    readonly property int sgn: (perfPage.store && perfPage.fmt)
                                               ? perfPage.fmt.decimalSign(perfPage.store.weekPnlPct)
                                               : 0
                    text: (perfPage.store && perfPage.fmt)
                          ? (perfPage.fmt.directionGlyph(perfPage.store.weekPnlPct) + " "
                             + perfPage.fmt.formatDecimal(perfPage.store.weekPnlPct, 2) + "%")
                          : "—"
                    color: Theme.signColor(sgn)
                    font.pixelSize: 18
                    font.bold: true
                    font.family: Theme.monoFont
                }
            }

            PanelCard {
                Layout.fillWidth: true
                title: "MAX DRAWDOWN"
                Text {
                    text: (perfPage.store && perfPage.fmt)
                          ? (perfPage.fmt.formatDecimal(perfPage.store.drawdownPct, 2) + "%")
                          : "—"
                    color: Theme.negative
                    font.pixelSize: 18
                    font.bold: true
                    font.family: Theme.monoFont
                }
            }
        }

        PanelCard {
            Layout.fillWidth: true
            Layout.fillHeight: true
            title: "EXECUTED TRADES"
            subtitle: (perfPage.store && perfPage.store.tradesModel)
                      ? (perfPage.store.tradesModel.count + " fills")
                      : ""

            RowLayout {
                Layout.fillWidth: true
                Button {
                    text: "↻ Refresh Trades"
                    onClicked: {
                        if (perfPage.commands && perfPage.store) {
                            perfPage.commands.refreshTrades(perfPage.store.activeVenue)
                        }
                    }
                }
                Item { Layout.fillWidth: true }
            }

            RowLayout {
                Layout.fillWidth: true
                spacing: 8
                Text { text: "Venue"; color: Theme.textSecondary; font.pixelSize: 11; font.bold: true; Layout.preferredWidth: 58 }
                Text { text: "Symbol"; color: Theme.textSecondary; font.pixelSize: 11; font.bold: true; Layout.preferredWidth: 88 }
                Text { text: "Side"; color: Theme.textSecondary; font.pixelSize: 11; font.bold: true; Layout.preferredWidth: 64 }
                Text { text: "Price"; color: Theme.textSecondary; font.pixelSize: 11; font.bold: true; Layout.preferredWidth: 100; horizontalAlignment: Text.AlignRight }
                Text { text: "Qty"; color: Theme.textSecondary; font.pixelSize: 11; font.bold: true; Layout.preferredWidth: 90; horizontalAlignment: Text.AlignRight }
                Text { text: "Fee"; color: Theme.textSecondary; font.pixelSize: 11; font.bold: true; Layout.preferredWidth: 90; horizontalAlignment: Text.AlignRight }
                Text { text: "Realized PnL"; color: Theme.textSecondary; font.pixelSize: 11; font.bold: true; Layout.fillWidth: true; horizontalAlignment: Text.AlignRight }
            }

            ListView {
                id: tradesList
                Layout.fillWidth: true
                Layout.fillHeight: true
                clip: true
                model: perfPage.store ? perfPage.store.tradesModel : null

                delegate: RowLayout {
                    width: tradesList.width
                    height: 30
                    spacing: 8

                    Text { text: model.venue.toUpperCase(); color: Theme.accent; font.pixelSize: 11; font.bold: true; Layout.preferredWidth: 58 }
                    Text { text: model.symbol; color: Theme.textPrimary; font.pixelSize: 12; font.bold: true; Layout.preferredWidth: 88 }
                    Text { text: model.side; color: Theme.textSecondary; font.pixelSize: 11; Layout.preferredWidth: 64 }
                    Text { text: perfPage.fmt ? perfPage.fmt.formatDecimal(model.price, 2) : model.price; color: Theme.textPrimary; font.pixelSize: 12; font.family: Theme.monoFont; Layout.preferredWidth: 100; horizontalAlignment: Text.AlignRight }
                    Text { text: perfPage.fmt ? perfPage.fmt.formatDecimal(model.qty) : model.qty; color: Theme.textPrimary; font.pixelSize: 12; font.family: Theme.monoFont; Layout.preferredWidth: 90; horizontalAlignment: Text.AlignRight }
                    Text { text: perfPage.fmt ? perfPage.fmt.formatDecimal(model.fee, 4) : model.fee; color: Theme.textSecondary; font.pixelSize: 12; font.family: Theme.monoFont; Layout.preferredWidth: 90; horizontalAlignment: Text.AlignRight }
                    Text {
                        readonly property int sgn: perfPage.fmt ? perfPage.fmt.decimalSign(model.realizedPnl) : 0
                        text: (perfPage.fmt ? perfPage.fmt.directionGlyph(model.realizedPnl) : "") + " "
                              + (perfPage.fmt ? perfPage.fmt.formatDecimal(model.realizedPnl, 2) : model.realizedPnl)
                        color: Theme.signColor(sgn)
                        font.pixelSize: 12
                        font.family: Theme.monoFont
                        Layout.fillWidth: true
                        horizontalAlignment: Text.AlignRight
                    }
                }
            }

            Text {
                visible: !perfPage.store || !perfPage.store.tradesModel || perfPage.store.tradesModel.count === 0
                text: "No executed trades returned."
                color: Theme.textSecondary
                font.pixelSize: 12
            }
        }
    }
}
