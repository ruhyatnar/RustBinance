import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import BinanceBot

Item {
    id: logsPage

    property var store
    property string selectedLevel: "ALL"

    readonly property var levels: ["ALL", "info", "warn", "error", "critical"]

    ColumnLayout {
        anchors.fill: parent
        spacing: 12

        RowLayout {
            Layout.fillWidth: true
            spacing: 8

            Repeater {
                model: logsPage.levels
                delegate: Button {
                    text: modelData.toUpperCase()
                    highlighted: logsPage.selectedLevel === modelData
                    onClicked: logsPage.selectedLevel = modelData
                }
            }

            TextField {
                id: searchBox
                placeholderText: "Filter event kind, symbol, or message…"
                Layout.fillWidth: true
            }
        }

        EventList {
            Layout.fillWidth: true
            Layout.fillHeight: true
            eventsModel: logsPage.store ? logsPage.store.eventsModel : null
            levelFilter: logsPage.selectedLevel
            searchQuery: searchBox.text
            preferredListHeight: 420
        }
    }
}
