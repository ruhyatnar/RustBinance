import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import BinanceBot

Item {
    id: posPage

    property var store
    property var commands
    property var fmt

    ColumnLayout {
        anchors.fill: parent
        spacing: 12

        PanelCard {
            Layout.fillWidth: true
            Layout.fillHeight: true
            title: "OPEN POSITIONS"
            subtitle: (posPage.store && posPage.store.positionsModel)
                      ? (posPage.store.positionsModel.count + " active")
                      : ""

            RowLayout {
                Layout.fillWidth: true
                spacing: 8
                Text { text: "Venue"; color: Theme.textSecondary; font.pixelSize: 11; font.bold: true; Layout.preferredWidth: 54 }
                Text { text: "Symbol"; color: Theme.textSecondary; font.pixelSize: 11; font.bold: true; Layout.preferredWidth: 84 }
                Text { text: "Strategy"; color: Theme.textSecondary; font.pixelSize: 11; font.bold: true; Layout.preferredWidth: 110 }
                Text { text: "State"; color: Theme.textSecondary; font.pixelSize: 11; font.bold: true; Layout.preferredWidth: 100 }
                Text { text: "Qty"; color: Theme.textSecondary; font.pixelSize: 11; font.bold: true; Layout.preferredWidth: 74; horizontalAlignment: Text.AlignRight }
                Text { text: "Entry"; color: Theme.textSecondary; font.pixelSize: 11; font.bold: true; Layout.preferredWidth: 88; horizontalAlignment: Text.AlignRight }
                Text { text: "Mark"; color: Theme.textSecondary; font.pixelSize: 11; font.bold: true; Layout.preferredWidth: 88; horizontalAlignment: Text.AlignRight }
                Text { text: "Stop / TP1 / TP2"; color: Theme.textSecondary; font.pixelSize: 11; font.bold: true; Layout.fillWidth: true }
                Text { text: "PnL (R / Quote)"; color: Theme.textSecondary; font.pixelSize: 11; font.bold: true; Layout.preferredWidth: 140; horizontalAlignment: Text.AlignRight }
                Text { visible: posPage.store ? posPage.store.canMutate : false; text: "Action"; color: Theme.textSecondary; font.pixelSize: 11; font.bold: true; Layout.preferredWidth: 68 }
            }

            ListView {
                id: posListView
                Layout.fillWidth: true
                Layout.fillHeight: true
                clip: true
                model: posPage.store ? posPage.store.positionsModel : null

                delegate: Rectangle {
                    width: posListView.width
                    height: 34
                    color: (index % 2 === 0) ? "transparent" : Theme.surfaceAltBg
                    radius: 4

                    RowLayout {
                        anchors.fill: parent
                        anchors.leftMargin: 4
                        anchors.rightMargin: 4
                        spacing: 8

                        Text { text: model.venue.toUpperCase(); color: Theme.accent; font.pixelSize: 11; font.bold: true; Layout.preferredWidth: 54 }
                        Text { text: model.symbol; color: Theme.textPrimary; font.pixelSize: 12; font.bold: true; Layout.preferredWidth: 84 }
                        Text { text: model.strategy; color: Theme.textSecondary; font.pixelSize: 11; Layout.preferredWidth: 110; elide: Text.ElideRight }
                        Text { text: model.state; color: Theme.textPrimary; font.pixelSize: 11; Layout.preferredWidth: 100; elide: Text.ElideRight }
                        Text { text: posPage.fmt ? posPage.fmt.formatDecimal(model.qty) : model.qty; color: Theme.textPrimary; font.pixelSize: 12; font.family: Theme.monoFont; Layout.preferredWidth: 74; horizontalAlignment: Text.AlignRight }
                        Text { text: posPage.fmt ? posPage.fmt.formatPrice(model.entry) : model.entry; color: Theme.textPrimary; font.pixelSize: 12; font.family: Theme.monoFont; Layout.preferredWidth: 88; horizontalAlignment: Text.AlignRight }
                        Text { text: posPage.fmt ? posPage.fmt.formatPrice(model.mark) : model.mark; color: Theme.textPrimary; font.pixelSize: 12; font.family: Theme.monoFont; Layout.preferredWidth: 88; horizontalAlignment: Text.AlignRight }
                        Text {
                            text: (posPage.fmt ? posPage.fmt.formatPrice(model.stop) : model.stop)
                                  + (model.beDone ? " (BE)" : "")
                                  + " / " + (posPage.fmt ? posPage.fmt.formatPrice(model.tp1) : model.tp1)
                                  + " / " + (posPage.fmt ? posPage.fmt.formatPrice(model.tp2) : model.tp2)
                            color: Theme.textSecondary
                            font.pixelSize: 11
                            font.family: Theme.monoFont
                            Layout.fillWidth: true
                            elide: Text.ElideRight
                        }
                        Text {
                            readonly property int sgn: posPage.fmt ? posPage.fmt.decimalSign(model.unrealizedQuote) : 0
                            text: (posPage.fmt ? posPage.fmt.directionGlyph(model.unrealizedQuote) : "") + " "
                                  + (posPage.fmt ? posPage.fmt.formatDecimal(model.profitR, 2) : model.profitR) + "R ("
                                  + (posPage.fmt ? posPage.fmt.formatDecimal(model.unrealizedQuote, 2) : model.unrealizedQuote) + ")"
                            color: Theme.signColor(sgn)
                            font.pixelSize: 12
                            font.family: Theme.monoFont
                            Layout.preferredWidth: 140
                            horizontalAlignment: Text.AlignRight
                        }
                        Button {
                            visible: posPage.store ? posPage.store.canMutate : false
                            text: "Close"
                            Layout.preferredWidth: 68
                            onClicked: if (posPage.commands) posPage.commands.closePosition(model.positionId)
                        }
                    }
                }
            }
        }

        PanelCard {
            Layout.fillWidth: true
            Layout.preferredHeight: 220
            title: "WORKING ORDERS (ENTRY & PROTECTION)"
            subtitle: (posPage.store && posPage.store.ordersModel)
                      ? (posPage.store.ordersModel.count + " orders")
                      : ""

            ListView {
                id: ordListView
                Layout.fillWidth: true
                Layout.fillHeight: true
                clip: true
                model: posPage.store ? posPage.store.ordersModel : null

                delegate: RowLayout {
                    width: ordListView.width
                    height: 28
                    spacing: 8

                    Text { text: model.venue.toUpperCase(); color: Theme.accent; font.pixelSize: 11; font.bold: true; Layout.preferredWidth: 54 }
                    Text { text: model.symbol; color: Theme.textPrimary; font.pixelSize: 12; font.bold: true; Layout.preferredWidth: 84 }
                    Text { text: model.side + " · " + model.kind; color: Theme.textSecondary; font.pixelSize: 11; Layout.preferredWidth: 150 }
                    Text { text: model.purpose; color: Theme.warning; font.pixelSize: 11; font.bold: true; Layout.preferredWidth: 90 }
                    Text { text: model.status; color: Theme.textPrimary; font.pixelSize: 11; Layout.preferredWidth: 90 }
                    Text {
                        text: (model.price.length > 0 && posPage.fmt)
                              ? ("@ " + posPage.fmt.formatPrice(model.price))
                              : "MARKET"
                        color: Theme.textPrimary
                        font.pixelSize: 12
                        font.family: Theme.monoFont
                        Layout.preferredWidth: 120
                    }
                    Text {
                        text: "Filled " + (posPage.fmt ? posPage.fmt.formatDecimal(model.filledQty) : model.filledQty)
                              + " / " + (posPage.fmt ? posPage.fmt.formatDecimal(model.qty) : model.qty)
                        color: Theme.textSecondary
                        font.pixelSize: 11
                        font.family: Theme.monoFont
                        Layout.fillWidth: true
                    }
                }
            }

            Text {
                visible: !posPage.store || !posPage.store.ordersModel || posPage.store.ordersModel.count === 0
                text: "No open orders."
                color: Theme.textSecondary
                font.pixelSize: 12
            }
        }
    }
}
