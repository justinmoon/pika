package com.pika.app.ui

object PeerKeyValidator {
    fun isValidPeer(input: String): Boolean {
        if (isHexPubkey(input)) return true
        if (isNpub(input)) return true
        return false
    }

    private fun isHexPubkey(s: String): Boolean {
        if (s.length != 64) return false
        return s.all { ch ->
            (ch in '0'..'9') || (ch in 'a'..'f') || (ch in 'A'..'F')
        }
    }

    private fun isNpub(raw: String): Boolean {
        val s = raw.lowercase()
        if (!s.startsWith("npub1")) return false
        val payload = s.substring(5)
        if (payload.length < 10) return false

        // bech32 charset: qpzry9x8gf2tvdw0s3jn54khce6mua7l
        val allowed = "qpzry9x8gf2tvdw0s3jn54khce6mua7l".toSet()
        return payload.all { it in allowed }
    }
}

