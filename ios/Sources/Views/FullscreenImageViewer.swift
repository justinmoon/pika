import SwiftUI

extension ChatMediaAttachment: Identifiable {
    public var id: String { originalHashHex }
}

struct FullscreenImageViewer: View {
    let attachments: [ChatMediaAttachment]
    @State var currentId: String
    @Environment(\.dismiss) private var dismiss
    @State private var dragOffset: CGSize = .zero
    @State private var backgroundOpacity: Double = 1.0

    init(attachment: ChatMediaAttachment) {
        self.attachments = [attachment]
        self._currentId = State(initialValue: attachment.id)
    }

    init(attachments: [ChatMediaAttachment], selected: ChatMediaAttachment) {
        self.attachments = attachments
        self._currentId = State(initialValue: selected.id)
    }

    private var currentAttachment: ChatMediaAttachment? {
        attachments.first { $0.id == currentId } ?? attachments.first
    }

    var body: some View {
        let dragProgress = min(abs(dragOffset.height) / 300, 1.0)

        NavigationStack {
            GeometryReader { geo in
                ZStack {
                    Color.black
                        .ignoresSafeArea()

                    TabView(selection: $currentId) {
                        ForEach(attachments) { attachment in
                            imageContent(attachment: attachment, geo: geo)
                                .tag(attachment.id)
                        }
                    }
                    .tabViewStyle(.page(indexDisplayMode: attachments.count > 1 ? .automatic : .never))
                    .offset(y: dragOffset.height)
                    .scaleEffect(1.0 - dragProgress * 0.2)
                    .gesture(
                        DragGesture(minimumDistance: 20)
                            .onChanged { value in
                                // Only respond to vertical drags
                                if abs(value.translation.height) > abs(value.translation.width) {
                                    dragOffset = CGSize(width: 0, height: value.translation.height)
                                    let progress = min(abs(value.translation.height) / 300, 1.0)
                                    backgroundOpacity = 1.0 - progress
                                }
                            }
                            .onEnded { value in
                                if abs(value.translation.height) > 100 {
                                    dismiss()
                                } else {
                                    withAnimation(.spring(response: 0.3, dampingFraction: 0.8)) {
                                        dragOffset = .zero
                                        backgroundOpacity = 1.0
                                    }
                                }
                            }
                    )
                }
                .opacity(backgroundOpacity)
            }
            .navigationTitle(currentAttachment?.filename ?? "")
            .navigationBarTitleDisplayMode(.inline)
            .toolbarColorScheme(.dark, for: .navigationBar)
            .toolbarBackground(.visible, for: .navigationBar)
            .toolbarBackground(Color.black, for: .navigationBar)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button {
                        dismiss()
                    } label: {
                        Image(systemName: "xmark")
                    }
                }
                ToolbarItem(placement: .topBarTrailing) {
                    ShareLink(item: URL(fileURLWithPath: currentAttachment?.localPath ?? "")) {
                        Image(systemName: "square.and.arrow.up")
                    }
                }
            }
        }
    }

    @ViewBuilder
    private func imageContent(attachment: ChatMediaAttachment, geo: GeometryProxy) -> some View {
        if let localPath = attachment.localPath {
            CachedAsyncImage(url: URL(fileURLWithPath: localPath)) { image in
                image
                    .resizable()
                    .scaledToFit()
                    .frame(maxWidth: geo.size.width, maxHeight: geo.size.height)
            } placeholder: {
                ProgressView()
                    .tint(.white)
            }
        }
    }
}
