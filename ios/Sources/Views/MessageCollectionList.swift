import SwiftUI
import UIKit

/// A UICollectionView-based message transcript for chat.
///
/// The collection view uses a normal top-to-bottom layout so scroll math,
/// sticky-bottom detection, and chrome reserves stay aligned with UIKit.
struct MessageCollectionList: UIViewRepresentable {
    struct ScrollRequest: Equatable {
        enum Action: Equatable {
            case scrollToBottom(animated: Bool)
        }

        let id: Int
        let action: Action
    }

    struct ContentState: Equatable {
        let chat: ChatViewState
        let activeReactionMessageId: String?
    }

    let rows: [ChatView.ChatTimelineRow]
    let chat: ChatViewState
    let messagesById: [String: ChatMessage]
    let isGroup: Bool

    let onSendMessage: @MainActor (String, String?) -> Void
    var onTapSender: (@MainActor (String) -> Void)?
    var onReact: (@MainActor (String, String) -> Void)?
    var onDownloadMedia: ((String, String) -> Void)?
    var onTapImage: (([ChatMediaAttachment], ChatMediaAttachment) -> Void)?
    var onHypernoteAction: ((String, String, [String: String]) -> Void)?
    var onLongPressMessage: ((ChatMessage, CGRect) -> Void)?
    var onRetryMessage: ((String) -> Void)?
    var onLoadOlderMessages: (() -> Void)?

    let viewportMetrics: MessageCollectionViewportMetrics
    @Binding var followsBottom: Bool
    var activeReactionMessageId: String?
    var scrollRequest: ScrollRequest?

    private var contentState: ContentState {
        ContentState(chat: chat, activeReactionMessageId: activeReactionMessageId)
    }

    func makeCoordinator() -> Coordinator {
        Coordinator(parent: self)
    }

    func makeUIView(context: Context) -> UICollectionView {
        let layout = MessageCollectionList.makeLayout()
        let collectionView = BoundsAwareCollectionView(frame: .zero, collectionViewLayout: layout)
        collectionView.backgroundColor = .clear
        collectionView.contentInsetAdjustmentBehavior = .never
        collectionView.alwaysBounceVertical = true
        collectionView.keyboardDismissMode = .interactive
        collectionView.delegate = context.coordinator
        collectionView.showsVerticalScrollIndicator = true
        collectionView.alwaysBounceHorizontal = false
        collectionView.onBoundsSizeChange = { [weak coordinator = context.coordinator] _ in
            coordinator?.handleBoundsSizeChange()
        }
        context.coordinator.collectionView = collectionView
        context.coordinator.lastContentState = contentState

        let registration = UICollectionView.CellRegistration<UICollectionViewCell, String> {
            [weak coordinator = context.coordinator] cell, _, itemID in
            guard let coordinator, let row = coordinator.rowsByID[itemID] else { return }
            var background = UIBackgroundConfiguration.clear()
            background.backgroundColor = .clear
            cell.backgroundConfiguration = background
            cell.contentConfiguration = UIHostingConfiguration {
                coordinator.rowContent(for: row, parent: coordinator.parent)
            }
            .minSize(width: 0, height: 0)
            .margins(.all, 0)
        }

        let dataSource = UICollectionViewDiffableDataSource<Int, String>(collectionView: collectionView) {
            collectionView, indexPath, itemID in
            collectionView.dequeueConfiguredReusableCell(
                using: registration,
                for: indexPath,
                item: itemID
            )
        }
        context.coordinator.dataSource = dataSource

        let renderedRows = buildRenderedRows()
        context.coordinator.applyRows(renderedRows, animated: false)
        context.coordinator.applyViewportMetricsIfNeeded(viewportMetrics)
        context.coordinator.scrollToBottom(animated: false)

        return collectionView
    }

    func updateUIView(_ collectionView: UICollectionView, context: Context) {
        let coordinator = context.coordinator
        coordinator.parent = self
        let newRows = buildRenderedRows()
        let newIDs = newRows.map(\.id)
        let updateKind = MessageCollectionLayout.classifyUpdate(oldIDs: coordinator.currentIDs, newIDs: newIDs)
        let anchor = followsBottom ? nil : coordinator.captureTopAnchor()
        let contentChanged = coordinator.lastContentState != contentState
        coordinator.lastContentState = contentState
        let viewportChanged = coordinator.applyViewportMetricsIfNeeded(viewportMetrics)
        let pendingScrollRequest = scrollRequest.flatMap { request in
            coordinator.consumeScrollRequestIfNeeded(request)
        }

        let completion = {
            if let pendingScrollRequest {
                coordinator.handle(scrollRequest: pendingScrollRequest)
            } else if coordinator.parent.followsBottom {
                let animateToBottom = updateKind == .tailMutation
                coordinator.scrollToBottom(animated: animateToBottom)
            } else if let anchor {
                coordinator.restore(anchor: anchor)
            }
        }

        switch updateKind {
        case .reconfigureOnly:
            let didApplyVisibleRefresh = contentChanged
                ? coordinator.reconfigureVisibleRows(with: newRows, completion: completion)
                : false
            if !didApplyVisibleRefresh && (viewportChanged || pendingScrollRequest != nil) {
                completion()
            }
        case .tailMutation, .structural:
            let animateDifferences = followsBottom && updateKind == .tailMutation
            coordinator.applyRows(newRows, animated: animateDifferences, completion: completion)
        }
    }

    private static func makeLayout() -> UICollectionViewLayout {
        let itemSize = NSCollectionLayoutSize(
            widthDimension: .fractionalWidth(1.0),
            heightDimension: .estimated(44)
        )
        let item = NSCollectionLayoutItem(layoutSize: itemSize)
        let group = NSCollectionLayoutGroup.vertical(layoutSize: itemSize, subitems: [item])
        let section = NSCollectionLayoutSection(group: group)
        section.interGroupSpacing = 0
        return UICollectionViewCompositionalLayout(section: section)
    }

    private func buildRenderedRows() -> [RenderedRow] {
        var rendered = rows.map(RenderedRow.timeline)
        if !chat.typingMembers.isEmpty {
            rendered.append(.typing)
        }
        return rendered
    }

    final class Coordinator: NSObject, UICollectionViewDelegate {
        var parent: MessageCollectionList
        var dataSource: UICollectionViewDiffableDataSource<Int, String>?
        var rowsByID: [String: RenderedRow] = [:]
        var currentIDs: [String] = []
        weak var collectionView: UICollectionView?
        private var requestedOldestId: String?
        private var lastAppliedViewportMetrics: MessageCollectionViewportMetrics?
        private var lastHandledScrollRequestID: Int?
        var lastContentState: ContentState?

        init(parent: MessageCollectionList) {
            self.parent = parent
        }

        func applyRows(_ rows: [RenderedRow], animated: Bool, completion: (() -> Void)? = nil) {
            currentIDs = rows.map(\.id)
            rowsByID = Dictionary(uniqueKeysWithValues: rows.map { ($0.id, $0) })

            var snapshot = NSDiffableDataSourceSnapshot<Int, String>()
            snapshot.appendSections([0])
            snapshot.appendItems(rows.map(\.id), toSection: 0)
            dataSource?.apply(snapshot, animatingDifferences: animated) {
                completion?()
            }
        }

        @discardableResult
        func reconfigureVisibleRows(with rows: [RenderedRow], completion: (() -> Void)? = nil) -> Bool {
            currentIDs = rows.map(\.id)
            rowsByID = Dictionary(uniqueKeysWithValues: rows.map { ($0.id, $0) })

            guard let dataSource else { return false }
            let visibleIDs = visibleItemIDs()
            guard !visibleIDs.isEmpty else { return false }

            var snapshot = dataSource.snapshot()
            snapshot.reconfigureItems(visibleIDs)
            dataSource.apply(snapshot, animatingDifferences: false) {
                completion?()
            }
            return true
        }

        func scrollToBottom(animated: Bool) {
            guard let collectionView else { return }
            collectionView.layoutIfNeeded()
            collectionView.setContentOffset(
                MessageCollectionLayout.bottomContentOffset(
                    contentHeight: collectionView.contentSize.height,
                    boundsHeight: collectionView.bounds.height,
                    adjustedInsets: collectionView.adjustedContentInset
                ),
                animated: animated
            )
        }

        @discardableResult
        func applyViewportMetricsIfNeeded(_ viewportMetrics: MessageCollectionViewportMetrics) -> Bool {
            guard let collectionView else { return false }
            guard viewportMetrics != lastAppliedViewportMetrics else { return false }
            lastAppliedViewportMetrics = viewportMetrics
            collectionView.contentInset = viewportMetrics.contentInset
            collectionView.scrollIndicatorInsets = viewportMetrics.scrollIndicatorInsets
            return true
        }

        func consumeScrollRequestIfNeeded(_ request: ScrollRequest) -> ScrollRequest? {
            guard request.id != lastHandledScrollRequestID else { return nil }
            lastHandledScrollRequestID = request.id
            return request
        }

        func handle(scrollRequest: ScrollRequest) {
            switch scrollRequest.action {
            case .scrollToBottom(let animated):
                scrollToBottom(animated: animated)
            }
        }

        func handleBoundsSizeChange() {
            guard parent.followsBottom else { return }
            scrollToBottom(animated: false)
        }

        func captureTopAnchor() -> ScrollAnchor? {
            guard let collectionView,
                  let dataSource,
                  let indexPath = collectionView.indexPathsForVisibleItems.sorted(by: indexPathSort).first,
                  let itemID = dataSource.itemIdentifier(for: indexPath),
                  let attributes = collectionView.layoutAttributesForItem(at: indexPath)
            else { return nil }

            return ScrollAnchor(
                itemID: itemID,
                distanceFromContentOffset: attributes.frame.minY - collectionView.contentOffset.y
            )
        }

        func restore(anchor: ScrollAnchor) {
            guard let collectionView,
                  let dataSource,
                  let indexPath = dataSource.indexPath(for: anchor.itemID)
            else { return }

            collectionView.layoutIfNeeded()
            collectionView.scrollToItem(at: indexPath, at: .top, animated: false)
            collectionView.layoutIfNeeded()

            guard let attributes = collectionView.layoutAttributesForItem(at: indexPath) else { return }

            let minOffsetY = -collectionView.adjustedContentInset.top
            let maxOffsetY = max(
                minOffsetY,
                collectionView.contentSize.height - collectionView.bounds.height + collectionView.adjustedContentInset.bottom
            )
            let targetY = min(
                max(attributes.frame.minY - anchor.distanceFromContentOffset, minOffsetY),
                maxOffsetY
            )
            collectionView.setContentOffset(CGPoint(x: 0, y: targetY), animated: false)
        }

        func collectionView(
            _ collectionView: UICollectionView,
            willDisplay cell: UICollectionViewCell,
            forItemAt indexPath: IndexPath
        ) {
            guard indexPath.item <= 2 else { return }
            guard parent.chat.canLoadOlder else { return }

            let oldestMessageId = parent.chat.messages.first?.id
            guard let oldestMessageId, oldestMessageId != requestedOldestId else { return }
            requestedOldestId = oldestMessageId
            parent.onLoadOlderMessages?()
        }

        func scrollViewDidScroll(_ scrollView: UIScrollView) {
            let nearBottom = MessageCollectionLayout.isNearBottom(
                contentOffsetY: scrollView.contentOffset.y,
                boundsHeight: scrollView.bounds.height,
                contentHeight: scrollView.contentSize.height,
                adjustedInsets: scrollView.adjustedContentInset
            )
            if nearBottom != parent.followsBottom {
                DispatchQueue.main.async {
                    self.parent.followsBottom = nearBottom
                }
            }
        }

        @ViewBuilder
        func rowContent(for row: RenderedRow, parent: MessageCollectionList) -> some View {
            switch row {
            case .typing:
                TypingIndicatorRow(
                    typingMembers: parent.chat.typingMembers,
                    members: parent.chat.members
                )
                .padding(.horizontal, 12)
                .padding(.vertical, 4)

            case .timeline(let timelineRow):
                Group {
                    switch timelineRow {
                    case .messageGroup(let group):
                        MessageGroupRow(
                            group: group,
                            showSender: parent.isGroup,
                            onSendMessage: parent.onSendMessage,
                            replyTargetsById: parent.messagesById,
                            onTapSender: parent.onTapSender,
                            onJumpToMessage: { [self] messageID in
                                jumpToMessage(messageID)
                            },
                            onReact: parent.onReact,
                            activeReactionMessageId: .constant(parent.activeReactionMessageId),
                            onLongPressMessage: parent.onLongPressMessage,
                            onDownloadMedia: parent.onDownloadMedia,
                            onTapImage: parent.onTapImage,
                            onHypernoteAction: parent.onHypernoteAction,
                            onRetryMessage: parent.onRetryMessage
                        )
                    case .unreadDivider:
                        UnreadDividerRow()
                    case .callEvent(let event):
                        CallTimelineEventRow(event: event)
                    }
                }
                .padding(.horizontal, 12)
                .padding(.vertical, 4)
            }
        }

        func jumpToMessage(_ messageID: String) {
            guard let dataSource,
                  let collectionView else { return }

            let snapshot = dataSource.snapshot()
            guard let rowID = snapshot.itemIdentifiers.first(where: { rowID in
                guard let row = rowsByID[rowID],
                      case .timeline(let timelineRow) = row,
                      case .messageGroup(let group) = timelineRow
                else { return false }

                return group.messages.contains { $0.id == messageID }
            }),
            let indexPath = dataSource.indexPath(for: rowID)
            else { return }

            collectionView.scrollToItem(at: indexPath, at: .centeredVertically, animated: true)
        }

        private func visibleItemIDs() -> [String] {
            guard let collectionView, let dataSource else { return [] }
            return collectionView.indexPathsForVisibleItems
                .sorted(by: indexPathSort)
                .compactMap { dataSource.itemIdentifier(for: $0) }
        }

        private func indexPathSort(_ lhs: IndexPath, _ rhs: IndexPath) -> Bool {
            if lhs.section == rhs.section {
                return lhs.item < rhs.item
            }
            return lhs.section < rhs.section
        }
    }
}

private final class BoundsAwareCollectionView: UICollectionView {
    var onBoundsSizeChange: ((CGSize) -> Void)?
    private var lastReportedSize: CGSize = .zero

    override func layoutSubviews() {
        super.layoutSubviews()
        guard bounds.size != lastReportedSize else { return }
        lastReportedSize = bounds.size
        onBoundsSizeChange?(bounds.size)
    }
}

struct ScrollAnchor {
    let itemID: String
    let distanceFromContentOffset: CGFloat
}

enum RenderedRow: Identifiable {
    case typing
    case timeline(ChatView.ChatTimelineRow)

    var id: String {
        switch self {
        case .typing:
            return "typing-indicator"
        case .timeline(let row):
            return row.id
        }
    }
}
