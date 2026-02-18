package com.justinmoon.rapture

import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import com.justinmoon.rapture.ui.MainApp
import com.justinmoon.rapture.ui.theme.AppTheme

class MainActivity : ComponentActivity() {
    private lateinit var manager: AppManager

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        manager = AppManager.getInstance(applicationContext)
        setContent {
            AppTheme {
                MainApp(manager = manager)
            }
        }
    }
}
