package com.pika.app

import android.content.Context

/**
 * Early-init hook for Rust `ndk-context` + `keyring-core` default store.
 *
 * UniFFI uses JNA, so we do not get `ndk-glue` to initialize context automatically.
 */
object Keyring {
    init {
        // Ensure the Rust shared library is loaded so JNI can resolve the symbol.
        System.loadLibrary("pika_core")
    }

    @JvmStatic external fun init(context: Context)
}

