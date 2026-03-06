import SwiftUI

struct NativeQuickActionButton: View {
    let title: String
    let systemImage: String
    var isPrimary: Bool = false
    var accessibilityIdentifier: String?
    let action: () -> Void

    @ViewBuilder
    var body: some View {
        if let accessibilityIdentifier, !accessibilityIdentifier.isEmpty {
            baseButton
                .accessibilityIdentifier(accessibilityIdentifier)
        } else {
            baseButton
        }
    }

    private var baseButton: some View {
        Button(action: action) {
            VStack(spacing: 10) {
                Image(systemName: systemImage)
                    .font(.body.weight(.semibold))
                    .foregroundStyle(isPrimary ? Color.white : Color.accentColor)
                    .frame(width: 50, height: 50)
                    .background(backgroundStyle, in: RoundedRectangle(cornerRadius: 18, style: .continuous))

                Text(title)
                    .font(.footnote)
                    .fontWeight(isPrimary ? .semibold : .regular)
                    .foregroundStyle(.primary)
                    .lineLimit(1)
                    .minimumScaleFactor(0.8)
            }
            .frame(maxWidth: .infinity)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
    }

    private var backgroundStyle: Color {
        isPrimary ? Color.accentColor : Color(.secondarySystemBackground)
    }
}

struct NativeBottomSearchField: View {
    let title: String
    @Binding var text: String

    var body: some View {
        HStack(spacing: 10) {
            Image(systemName: "magnifyingglass")
                .foregroundStyle(.secondary)
            TextField(title, text: $text)
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
        }
        .padding(.horizontal, 16)
        .frame(height: 52)
        .background(Color(.systemBackground), in: RoundedRectangle(cornerRadius: 26, style: .continuous))
        .overlay {
            RoundedRectangle(cornerRadius: 26, style: .continuous)
                .stroke(Color.black.opacity(0.03), lineWidth: 1)
        }
        .shadow(color: .black.opacity(0.03), radius: 12, y: 2)
    }
}
