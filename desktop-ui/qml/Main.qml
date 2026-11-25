import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import BinanceBot

ApplicationWindow {
    id: rootWin
    width: 1280
    height: 820
    minimumWidth: 1024
    minimumHeight: 680
    visible: true
    title: "Binance Trading Bot · " + (stateStore ? stateStore.buildMode.toUpperCase() : "PAPER")
    color: Theme.windowBg

    property int currentPageIndex: 0

    readonly property var navLabels: [
        "Cockpit",
        "Positions",
        "Signals",
        "Market",
        "Risk",
        "Performance",
        "Logs",
        "Settings",
        "System"
    ]

    onClosing: function(close) {
        if (notifier && notifier.hasSystemTray && !notifier.appQuitting) {
            close.accepted = false
            rootWin.visible = false
        }
    }

    Connections {
        target: stateStore
        function onSnapshotChanged() {
            if (stateStore.hasSnapshot && stateStore.klines.length === 0 && commandService) {
                commandService.refreshKlines(
                    stateStore.activeVenue,
                    stateStore.chartSymbol,
                    stateStore.chartTf,
                    120)
                commandService.refreshTrades(stateStore.activeVenue)
            }
        }
    }

    header: ColumnLayout {
        spacing: 0

        ModeBanner {
            Layout.fillWidth: true
            store: stateStore
            fmt: decimalFormat
            commandBusy: commandService ? commandService.busy : false
            onToggleControlsClicked: if (stateStore) stateStore.toggleControlsUnlocked()
            onPanicConfirmed: if (commandService) commandService.executeConfirmedAction("panic_flatten")
        }

        // Action feedback bar
        Rectangle {
            Layout.fillWidth: true
            height: 32
            visible: stateStore ? (stateStore.actionFeedback.length > 0) : false
            color: Theme.surfaceAltBg
            border.color: Theme.warning
            border.width: 1

            RowLayout {
                anchors.fill: parent
                anchors.leftMargin: 12
                anchors.rightMargin: 12
                spacing: 8

                Text {
                    text: stateStore ? stateStore.actionFeedback : ""
                    color: Theme.warning
                    font.pixelSize: 12
                    Layout.fillWidth: true
                    elide: Text.ElideRight
                }

                Button {
                    text: "Dismiss"
                    onClicked: if (stateStore) stateStore.clearActionFeedback()
                }
            }
        }

        // Top navigation bar (9 screens matching §11.2 & web-ui)
        Rectangle {
            Layout.fillWidth: true
            height: 38
            color: Theme.surfaceBg
            border.color: Theme.border
            border.width: 1

            RowLayout {
                anchors.fill: parent
                anchors.leftMargin: 8
                anchors.rightMargin: 8
                spacing: 4

                Repeater {
                    model: rootWin.navLabels
                    delegate: Button {
                        text: modelData
                        highlighted: rootWin.currentPageIndex === index
                        onClicked: rootWin.currentPageIndex = index
                    }
                }

                Item { Layout.fillWidth: true }

                VenueSwitcher {
                    activeVenue: stateStore ? stateStore.activeVenue : "spot"
                    spotState: stateStore ? stateStore.spotState : "Active"
                    futuresState: stateStore ? stateStore.futuresState : "Disabled"
                    readOnly: !(stateStore && stateStore.canMutate)
                    onRequestSwitch: function(targetVenue) {
                        if (stateStore && targetVenue !== stateStore.activeVenue) {
                            venueSwitchDialog.targetVenue = targetVenue
                            venueSwitchDialog.open()
                        }
                    }
                }
            }
        }
    }

    VenueSwitchDialog {
        id: venueSwitchDialog
        currentVenue: stateStore ? stateStore.activeVenue : "spot"
        engineMode: stateStore ? stateStore.engineMode : "Starting"
        openPositionsCount: stateStore ? stateStore.openPositionsCount : 0
        onConfirmSwitch: function(targetVenue, policy) {
            if (commandService) {
                commandService.switchVenue(targetVenue, policy)
            }
        }
    }

    StackLayout {
        anchors.fill: parent
        anchors.margins: 12
        currentIndex: rootWin.currentPageIndex

        CockpitPage {
            store: stateStore
            commands: commandService
            fmt: decimalFormat
        }
        PositionsPage {
            store: stateStore
            commands: commandService
            fmt: decimalFormat
        }
        SignalsPage {
            store: stateStore
            fmt: decimalFormat
        }
        MarketPage {
            store: stateStore
            commands: commandService
            fmt: decimalFormat
        }
        RiskPage {
            store: stateStore
            commands: commandService
            fmt: decimalFormat
        }
        PerformancePage {
            store: stateStore
            commands: commandService
            fmt: decimalFormat
        }
        LogsPage {
            store: stateStore
        }
        SettingsPage {
            store: stateStore
            commands: commandService
        }
        SystemPage {
            store: stateStore
            client: ipcClient
        }
    }
}
