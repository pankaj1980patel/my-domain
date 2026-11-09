package com.mydomain.android.ui

import androidx.compose.foundation.layout.heightIn

import android.content.SharedPreferences
import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.selection.selectable
import androidx.compose.foundation.selection.selectableGroup
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.unit.dp

/** One row per brand-kit theme; tapping switches the whole app at once and is
 *  remembered (ThemeStore). Self-contained, so any screen can host it. */
@Composable
fun ThemePicker(prefs: SharedPreferences) {
    Column(Modifier.selectableGroup(), verticalArrangement = Arrangement.spacedBy(8.dp)) {
        ThemeId.entries.forEach { id ->
            val selected = ThemeStore.current == id
            val sw = id.tokens
            Surface(
                shape = MaterialTheme.shapes.small,
                color = T.surface2,
                border = BorderStroke(if (selected) 2.dp else 1.dp, if (selected) T.accent else T.border),
                modifier = Modifier.fillMaxWidth().heightIn(min = 48.dp).selectable(
                    selected = selected, role = Role.RadioButton,
                    onClick = { ThemeStore.set(prefs, id) },
                ),
            ) {
                Row(Modifier.padding(12.dp), verticalAlignment = Alignment.CenterVertically) {
                    // Swatches: background, surface, primary, accent of that theme.
                    Row(horizontalArrangement = Arrangement.spacedBy(4.dp)) {
                        listOf(sw.bg, sw.bar.copy(alpha = 1f), sw.primary, sw.accent).forEach { c ->
                            Box(Modifier.size(18.dp).background(c, CircleShape).border(1.dp, T.borderStrong, CircleShape))
                        }
                    }
                    Spacer(Modifier.width(12.dp))
                    Column(Modifier.weight(1f)) {
                        Text(stringResource(id.label), style = MaterialTheme.typography.titleMedium, color = T.text)
                        Text(stringResource(id.desc), style = MaterialTheme.typography.bodySmall, color = T.text3)
                    }
                }
            }
        }
    }
}
