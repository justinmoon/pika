package com.justinmoon.rapture.ui

import androidx.compose.foundation.layout.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import com.justinmoon.rapture.AppManager
import com.justinmoon.rapture.rust.AppAction

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun MainApp(manager: AppManager) {
    var nameInput by remember { mutableStateOf("") }
    val state = manager.state

    Scaffold(
        topBar = {
            TopAppBar(title = { Text("Rapture") })
        },
    ) { padding ->
        Column(
            modifier = Modifier
                .fillMaxSize()
                .padding(padding)
                .padding(20.dp),
            horizontalAlignment = Alignment.CenterHorizontally,
            verticalArrangement = Arrangement.spacedBy(16.dp),
        ) {
            Text(
                state.greeting,
                style = MaterialTheme.typography.headlineMedium,
            )

            OutlinedTextField(
                value = nameInput,
                onValueChange = { nameInput = it },
                label = { Text("Enter your name") },
                modifier = Modifier.fillMaxWidth(),
                singleLine = true,
            )

            Button(
                onClick = { manager.dispatch(AppAction.SetName(nameInput)) },
                modifier = Modifier.fillMaxWidth(),
            ) {
                Text("Greet")
            }
        }
    }
}
