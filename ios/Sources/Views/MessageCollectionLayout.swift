import UIKit

struct MessageCollectionViewportMetrics: Equatable {
    let jumpButtonBottomOffset: CGFloat
}

enum MessageCollectionRowID {
    static let typingIndicator = "typing-indicator"
}

enum MessageCollectionUpdateKind: Equatable {
    case reconfigureOnly
    case tailMutation
    case structural
}

enum MessageCollectionLayout {
    static func viewportMetrics(
        jumpButtonSpacing: CGFloat = 12
    ) -> MessageCollectionViewportMetrics {
        return MessageCollectionViewportMetrics(
            jumpButtonBottomOffset: jumpButtonSpacing
        )
    }

    static func effectiveContentInset(
        boundsHeight: CGFloat,
        contentHeight: CGFloat,
        bottomInset: CGFloat
    ) -> UIEdgeInsets {
        let availableHeight = max(0, boundsHeight - bottomInset)
        let extraTopInset = max(0, availableHeight - contentHeight)
        return UIEdgeInsets(
            top: extraTopInset,
            left: 0,
            bottom: bottomInset,
            right: 0
        )
    }

    static func classifyUpdate(oldIDs: [String], newIDs: [String]) -> MessageCollectionUpdateKind {
        guard oldIDs != newIDs else { return .reconfigureOnly }
        if oldIDs.isPrefix(of: newIDs) || newIDs.isPrefix(of: oldIDs) {
            return .tailMutation
        }
        return .structural
    }

    static func isNearBottom(
        contentOffsetY: CGFloat,
        boundsHeight: CGFloat,
        contentHeight: CGFloat,
        adjustedInsets: UIEdgeInsets,
        tolerance: CGFloat = 50
    ) -> Bool {
        let visibleBottom = contentOffsetY + boundsHeight - adjustedInsets.bottom
        return visibleBottom >= contentHeight - tolerance
    }

    static func bottomContentOffset(
        contentHeight: CGFloat,
        boundsHeight: CGFloat,
        adjustedInsets: UIEdgeInsets
    ) -> CGPoint {
        let minOffsetY = -adjustedInsets.top
        let maxOffsetY = max(minOffsetY, contentHeight - boundsHeight + adjustedInsets.bottom)
        return CGPoint(x: 0, y: maxOffsetY)
    }
}

private extension Array where Element: Equatable {
    func isPrefix(of other: [Element]) -> Bool {
        guard count <= other.count else { return false }
        return zip(self, other).allSatisfy(==)
    }
}
