import XCTest
@testable import Pika

final class MessageCollectionLayoutTests: XCTestCase {
    func testViewportMetricsShareChromeGeometryAcrossListAndJumpButton() {
        let metrics = MessageCollectionLayout.viewportMetrics(
            extraBottomSpacing: 20,
            jumpButtonSpacing: 12
        )

        XCTAssertEqual(metrics.contentInset.top, 0)
        XCTAssertEqual(metrics.contentInset.bottom, 20)
        XCTAssertEqual(metrics.scrollIndicatorInsets, .zero)
        XCTAssertEqual(metrics.jumpButtonBottomOffset, 12)
    }

    func testNearBottomUsesVisibleViewportBottom() {
        let insets = UIEdgeInsets(top: 30, left: 0, bottom: 106, right: 0)

        XCTAssertTrue(
            MessageCollectionLayout.isNearBottom(
                contentOffsetY: 900,
                boundsHeight: 500,
                contentHeight: 1300,
                adjustedInsets: insets
            )
        )
        XCTAssertFalse(
            MessageCollectionLayout.isNearBottom(
                contentOffsetY: 700,
                boundsHeight: 500,
                contentHeight: 1300,
                adjustedInsets: insets
            )
        )
    }

    func testBottomContentOffsetRespectsInsets() {
        let offset = MessageCollectionLayout.bottomContentOffset(
            contentHeight: 1300,
            boundsHeight: 500,
            adjustedInsets: UIEdgeInsets(top: 30, left: 0, bottom: 106, right: 0)
        )
        XCTAssertEqual(offset, CGPoint(x: 0, y: 906))
    }

    func testUpdateClassificationUsesTailMutationForAppendAndTrim() {
        XCTAssertEqual(
            MessageCollectionLayout.classifyUpdate(
                oldIDs: ["a", "b"],
                newIDs: ["a", "b", "c"]
            ),
            .tailMutation
        )
        XCTAssertEqual(
            MessageCollectionLayout.classifyUpdate(
                oldIDs: ["a", "b", "c"],
                newIDs: ["a", "b"]
            ),
            .tailMutation
        )
    }

    func testUpdateClassificationTreatsReshapesAsStructural() {
        XCTAssertEqual(
            MessageCollectionLayout.classifyUpdate(
                oldIDs: ["row-1", "row-2"],
                newIDs: ["row-0", "row-2"]
            ),
            .structural
        )
        XCTAssertEqual(
            MessageCollectionLayout.classifyUpdate(
                oldIDs: ["row-1", "row-2"],
                newIDs: ["row-1", "row-2"]
            ),
            .reconfigureOnly
        )
    }
}
