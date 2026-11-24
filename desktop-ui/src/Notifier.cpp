#include "Notifier.h"

#include <QAction>
#include <QApplication>
#include <QColor>
#include <QFont>
#include <QGuiApplication>
#include <QMenu>
#include <QPainter>
#include <QPixmap>
#include <QSystemTrayIcon>

#if HAVE_KF6
#include <KNotification>
#include <KStatusNotifierItem>
#endif

#include "CommandService.h"
#include "StateStore.h"

namespace botui {

Notifier::Notifier(StateStore* store, CommandService* commands, QObject* parent)
    : QObject(parent), m_store(store), m_commands(commands) {
    // Always construct the menu actions (when a QApplication is active) so action set is deterministic and auditable.
    if (qobject_cast<QApplication*>(QCoreApplication::instance()) != nullptr) {
        buildTrayMenu();
    }

    const QString platform = QGuiApplication::platformName();
    const bool isHeadless = (platform == QStringLiteral("offscreen") ||
                             platform == QStringLiteral("minimal"));

    if (!isHeadless && m_menu != nullptr) {
#if HAVE_KF6
        if (qEnvironmentVariableIsSet("DBUS_SESSION_BUS_ADDRESS")) {
            m_sni = new KStatusNotifierItem(QStringLiteral("binance-bot-ui"), this);
            m_sni->setCategory(KStatusNotifierItem::ApplicationStatus);
            m_sni->setStatus(KStatusNotifierItem::Active);
            m_sni->setStandardActionsEnabled(false);
            m_sni->setContextMenu(m_menu);
            connect(m_sni, &KStatusNotifierItem::activateRequested, this, [this](bool, const QPoint&) {
                emit openCockpitRequested();
            });
            m_hasSystemTray = true;
        }
#endif
        if (!m_hasSystemTray && QSystemTrayIcon::isSystemTrayAvailable()) {
            m_qtTray = new QSystemTrayIcon(this);
            m_qtTray->setContextMenu(m_menu);
            connect(m_qtTray, &QSystemTrayIcon::activated, this,
                    [this](QSystemTrayIcon::ActivationReason reason) {
                        if (reason == QSystemTrayIcon::Trigger ||
                            reason == QSystemTrayIcon::DoubleClick) {
                            emit openCockpitRequested();
                        }
                    });
            m_qtTray->show();
            m_hasSystemTray = true;
        }
    }

    if (m_store != nullptr) {
        connect(m_store, &StateStore::snapshotChanged, this, &Notifier::refreshTrayStatus);
        connect(m_store, &StateStore::connectionChanged, this, &Notifier::refreshTrayStatus);
        connect(m_store, &StateStore::criticalNotificationRequested, this, &Notifier::postNotification);
    }

    refreshTrayStatus();
}

Notifier::~Notifier() {
    delete m_menu;
    m_menu = nullptr;
}

void Notifier::buildTrayMenu() {
    // §11.4: Tray quick menu has Open Cockpit, Pause/Resume, and Quit UI — NEVER Panic.
    m_menu = new QMenu();
    m_openAction = m_menu->addAction(QStringLiteral("Open Cockpit"));
    m_menu->addSeparator();
    m_pauseAction = m_menu->addAction(QStringLiteral("Pause Entries"));
    m_resumeAction = m_menu->addAction(QStringLiteral("Resume Entries"));
    m_menu->addSeparator();
    m_quitAction = m_menu->addAction(QStringLiteral("Quit UI"));

    connect(m_openAction, &QAction::triggered, this, &Notifier::openCockpitRequested);
    connect(m_pauseAction, &QAction::triggered, this, [this]() {
        if (m_commands != nullptr) {
            m_commands->trayPause();
        }
    });
    connect(m_resumeAction, &QAction::triggered, this, [this]() {
        if (m_commands != nullptr) {
            m_commands->trayResume();
        }
    });
    connect(m_quitAction, &QAction::triggered, this, &Notifier::requestQuit);
}

QStringList Notifier::menuActionLabels() const {
    if (m_menu == nullptr) {
        return {
            QStringLiteral("Open Cockpit"),
            QStringLiteral("Pause Entries"),
            QStringLiteral("Resume Entries"),
            QStringLiteral("Quit UI"),
        };
    }
    QStringList out;
    for (const QAction* act : m_menu->actions()) {
        if (!act->isSeparator() && !act->text().isEmpty()) {
            out.push_back(act->text());
        }
    }
    return out;
}

QIcon Notifier::renderModeIcon(const QString& buildMode, const QString& engineMode) {
    QPixmap pix(64, 64);
    pix.fill(Qt::transparent);

    QPainter p(&pix);
    p.setRenderHint(QPainter::Antialiasing, true);

    const QString modeLower = buildMode.toLower();
    QColor bgColor(0x02, 0x84, 0xc7); // PAPER blue
    QString badgeLetter = QStringLiteral("P");
    if (modeLower == QStringLiteral("testnet")) {
        bgColor = QColor(0xd9, 0x77, 0x06); // TESTNET amber
        badgeLetter = QStringLiteral("T");
    } else if (modeLower == QStringLiteral("live")) {
        bgColor = QColor(0xdc, 0x26, 0x26); // LIVE red
        badgeLetter = QStringLiteral("L");
    }

    p.setPen(Qt::NoPen);
    p.setBrush(bgColor);
    p.drawRoundedRect(4, 4, 56, 56, 14, 14);

    if (modeLower == QStringLiteral("live")) {
        // High-visibility diagonal orange stripes for LIVE (§11.1)
        p.setPen(QPen(QColor(0xf9, 0x73, 0x16, 170), 5));
        p.drawLine(10, 54, 54, 10);
        p.drawLine(4, 36, 36, 4);
        p.drawLine(28, 60, 60, 28);
    }

    QFont font = p.font();
    font.setBold(true);
    font.setPixelSize(32);
    p.setFont(font);
    p.setPen(Qt::white);
    p.drawText(QRect(4, 4, 56, 56), Qt::AlignCenter, badgeLetter);

    // Status corner dot for EngineMode
    QColor dotColor(0x10, 0xb9, 0x81); // Ready emerald
    if (engineMode == QStringLiteral("Paused") || engineMode == QStringLiteral("ReduceOnly")) {
        dotColor = QColor(0xf5, 0x9e, 0x0b);
    } else if (engineMode == QStringLiteral("Halted") || engineMode == QStringLiteral("Killed")) {
        dotColor = QColor(0xef, 0x44, 0x44);
    } else if (engineMode == QStringLiteral("Starting") || engineMode == QStringLiteral("Syncing")) {
        dotColor = QColor(0x94, 0xa3, 0xb8);
    }

    p.setPen(QPen(QColor(0x0f, 0x17, 0x2a), 3));
    p.setBrush(dotColor);
    p.drawEllipse(42, 42, 18, 18);
    p.end();

    return QIcon(pix);
}

void Notifier::refreshTrayStatus() {
    const QString buildMode = (m_store != nullptr) ? m_store->buildMode() : QStringLiteral("paper");
    const QString engineMode = (m_store != nullptr) ? m_store->engineMode() : QStringLiteral("Starting");
    const bool isConn = (m_store != nullptr) &&
                        (m_store->connectionState() == QStringLiteral("connected"));

    if (m_pauseAction != nullptr) {
        m_pauseAction->setEnabled(isConn && engineMode == QStringLiteral("Ready"));
    }
    if (m_resumeAction != nullptr) {
        m_resumeAction->setEnabled(isConn && engineMode == QStringLiteral("Paused"));
    }

    if (!m_hasSystemTray) {
        return;
    }

    const QIcon icon = renderModeIcon(buildMode, engineMode);
    const QString tooltip = QStringLiteral("Binance Bot [%1 · %2]")
                                .arg(buildMode.toUpper(), engineMode);

#if HAVE_KF6
    if (m_sni != nullptr) {
        m_sni->setIconByPixmap(icon);
        m_sni->setToolTipIconByPixmap(icon);
        m_sni->setToolTipTitle(tooltip);
        m_sni->setToolTipSubTitle(isConn ? QStringLiteral("IPC Connected")
                                         : QStringLiteral("IPC Disconnected"));
    }
#endif
    if (m_qtTray != nullptr) {
        m_qtTray->setIcon(icon);
        m_qtTray->setToolTip(tooltip);
    }
}

void Notifier::postNotification(const QString& title, const QString& body, const QString& level) {
    if (!m_hasSystemTray) {
        return;
    }

#if HAVE_KF6
    if (m_sni != nullptr) {
        auto* notif = new KNotification(QStringLiteral("botAlert"), KNotification::CloseOnTimeout, this);
        notif->setComponentName(QStringLiteral("binance-bot-ui"));
        notif->setTitle(title);
        notif->setText(body);
        if (level == QStringLiteral("critical") || level == QStringLiteral("error")) {
            notif->setUrgency(KNotification::CriticalUrgency);
        } else {
            notif->setUrgency(KNotification::NormalUrgency);
        }
        notif->sendEvent();
        return;
    }
#endif

    if (m_qtTray != nullptr) {
        const auto iconType = (level == QStringLiteral("critical") || level == QStringLiteral("error"))
                                  ? QSystemTrayIcon::Critical
                                  : QSystemTrayIcon::Warning;
        m_qtTray->showMessage(title, body, iconType, 5000);
    }
}

void Notifier::requestQuit() {
    if (!m_appQuitting) {
        m_appQuitting = true;
        emit appQuittingChanged();
    }
    QCoreApplication::quit();
}

} // namespace botui
