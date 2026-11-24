import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import BinanceBot

Item {
    id: marketPage

    property var store
    property var commands
    property var fmt

    readonly property var timeframes: ["1m", "5m", "15m", "1h", "4h", "1d"]

    ColumnLayout {
        anchors.fill: parent
        spacing: 12

        RowLayout {
            Layout.fillWidth: true
            spacing: 8

            ComboBox {
                id: symbolBox
                model: marketPage.store ? marketPage.store.availableSymbols : ["BTCUSDT"]
                onActivated: function(idx) {
                    if (marketPage.store && idx >= 0) {
                        var chosen = symbolBox.textAt(idx)
                        marketPage.store.chartSymbol = chosen
                        if (marketPage.commands) {
                            marketPage.commands.refreshKlines(
                                marketPage.store.activeVenue,
                                chosen,
                                marketPage.store.chartTf,
                                160)
                        }
                    }
                }
            }

            Repeater {
                model: marketPage.timeframes
                delegate: Button {
                    text: modelData
                    highlighted: marketPage.store ? (marketPage.store.chartTf === modelData) : false
                    onClicked: {
                        if (marketPage.store) {
                            marketPage.store.chartTf = modelData
                            if (marketPage.commands) {
                                marketPage.commands.refreshKlines(
                                    marketPage.store.activeVenue,
                                    marketPage.store.chartSymbol,
                                    modelData,
                                    160)
                            }
                        }
                    }
                }
            }

            Item { Layout.fillWidth: true }
        }

        CandlesChart {
            Layout.fillWidth: true
            Layout.fillHeight: true
            symbol: marketPage.store ? marketPage.store.chartSymbol : "BTCUSDT"
            timeframe: marketPage.store ? marketPage.store.chartTf : "15m"
            bars: marketPage.store ? marketPage.store.klines : []
            positionOverlay: marketPage.store ? marketPage.store.activeChartPosition : ({})
            fmt: marketPage.fmt
            chartHeight: 320
        }

        DepthLadder {
            Layout.fillWidth: true
            ticksModel: marketPage.store ? marketPage.store.marketTicksModel : null
            fmt: marketPage.fmt
            onSymbolSelected: function(sym) {
                if (marketPage.store) {
                    marketPage.store.chartSymbol = sym
                    if (marketPage.commands) {
                        marketPage.commands.refreshKlines(
                            marketPage.store.activeVenue,
                            sym,
                            marketPage.store.chartTf,
                            160)
                    }
                }
            }
        }
    }
}
