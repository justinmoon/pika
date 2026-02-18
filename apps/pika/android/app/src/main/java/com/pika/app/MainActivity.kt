package com.pika.app

import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import com.pika.app.ui.PikaApp
import com.pika.app.ui.theme.PikaTheme

class MainActivity : ComponentActivity() {
    private lateinit var manager: AppManager

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)

        // Required by spec-v2: initialize Android keystore-backed keyring store once per process
        // before Rust constructs MDK encrypted SQLite storage.
        Keyring.init(applicationContext)

        manager = AppManager.getInstance(applicationContext)

        setContent {
            PikaTheme {
                PikaApp(manager = manager)
            }
        }
    }

    override fun onResume() {
        super.onResume()
        manager.onForeground()
    }
}
