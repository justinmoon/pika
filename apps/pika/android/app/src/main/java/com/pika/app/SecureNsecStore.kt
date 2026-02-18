package com.pika.app

import android.content.Context
import androidx.security.crypto.EncryptedSharedPreferences
import androidx.security.crypto.MasterKey

class SecureNsecStore(context: Context) {
    private val appContext = context.applicationContext

    private val prefs by lazy {
        val masterKey =
            MasterKey.Builder(appContext)
                .setKeyScheme(MasterKey.KeyScheme.AES256_GCM)
                .build()

        EncryptedSharedPreferences.create(
            appContext,
            "pika.secure",
            masterKey,
            EncryptedSharedPreferences.PrefKeyEncryptionScheme.AES256_SIV,
            EncryptedSharedPreferences.PrefValueEncryptionScheme.AES256_GCM,
        )
    }

    fun getNsec(): String? = prefs.getString(KEY_NSEC, null)

    fun setNsec(nsec: String) {
        prefs.edit().putString(KEY_NSEC, nsec).apply()
    }

    fun clearNsec() {
        prefs.edit().remove(KEY_NSEC).apply()
    }

    companion object {
        private const val KEY_NSEC = "nsec"
    }
}

