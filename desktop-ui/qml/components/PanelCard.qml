import QtQuick
import QtQuick.Layouts
import BinanceBot

Rectangle {
    id: card

    property string title: ""
    property string subtitle: ""
    default property alias content: bodyColumn.data

    color: Theme.surfaceBg
    border.color: Theme.border
    border.width: 1
    radius: 8
    implicitHeight: mainLayout.implicitHeight + 24

    ColumnLayout {
        id: mainLayout
        anchors.fill: parent
        anchors.margins: 12
        spacing: 8

        RowLayout {
            Layout.fillWidth: true
            visible: card.title.length > 0 || card.subtitle.length > 0

            Text {
                text: card.title
                color: Theme.textSecondary
                font.pixelSize: 11
                font.bold: true
                Layout.fillWidth: true
            }

            Text {
                text: card.subtitle
                color: Theme.textSecondary
                font.pixelSize: 11
                font.family: Theme.monoFont
                visible: card.subtitle.length > 0
            }
        }

        ColumnLayout {
            id: bodyColumn
            Layout.fillWidth: true
            Layout.fillHeight: true
            spacing: 6
        }
    }
}
