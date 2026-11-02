package com.mydomain.android.ui

import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Typography
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.Immutable
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.sp

// The same design tokens as the desktop (apps/desktop/ui/src/styles/tokens.css):
// white and soft greys, near-black text, one accent (the logo navy) for
// primary actions only. Light and dark follow the system setting.

@Immutable
data class Tokens(
    val bg: Color,
    val surface: Color,
    val surface2: Color,
    val border: Color,
    val text: Color,
    val text2: Color,
    val text3: Color,
    val accent: Color,
    val accentText: Color,
    val success: Color,
    val successSoft: Color,
    val warning: Color,
    val danger: Color,
    val dangerSoft: Color,
)

private val Light = Tokens(
    bg = Color(0xFFFAFAFA), surface = Color(0xFFFFFFFF), surface2 = Color(0xFFF4F4F5),
    border = Color(0xFFE4E4E7), text = Color(0xFF18181B), text2 = Color(0xFF52525B), text3 = Color(0xFF71717A),
    accent = Color(0xFF0F1A4F), accentText = Color.White,
    success = Color(0xFF15803D), successSoft = Color(0xFFF0FDF4), warning = Color(0xFFA16207),
    danger = Color(0xFFB91C1C), dangerSoft = Color(0xFFFEF2F2),
)

private val Dark = Tokens(
    bg = Color(0xFF0A0A0A), surface = Color(0xFF141414), surface2 = Color(0xFF1F1F1F),
    border = Color(0xFF2A2A2A), text = Color(0xFFFAFAFA), text2 = Color(0xFFA1A1AA), text3 = Color(0xFF8B8B93),
    accent = Color(0xFFFAFAFA), accentText = Color(0xFF0A0A0A),
    success = Color(0xFF4ADE80), successSoft = Color(0xFF0F1F14), warning = Color(0xFFFACC15),
    danger = Color(0xFFF87171), dangerSoft = Color(0xFF1F0F0F),
)

val LocalTokens = staticCompositionLocalOf { Light }

/** The current design tokens: `T.text`, `T.border`, ... */
val T: Tokens
    @Composable get() = LocalTokens.current

private val type = Typography(
    headlineSmall = TextStyle(fontSize = 22.sp, fontWeight = FontWeight.SemiBold),
    titleLarge = TextStyle(fontSize = 18.sp, fontWeight = FontWeight.SemiBold),
    titleMedium = TextStyle(fontSize = 15.sp, fontWeight = FontWeight.SemiBold),
    bodyLarge = TextStyle(fontSize = 15.sp),
    bodyMedium = TextStyle(fontSize = 14.sp),
    bodySmall = TextStyle(fontSize = 13.sp),
    labelLarge = TextStyle(fontSize = 14.sp, fontWeight = FontWeight.Medium),
    labelMedium = TextStyle(fontSize = 12.sp, fontWeight = FontWeight.Medium),
)

@Composable
fun MdTheme(content: @Composable () -> Unit) {
    val t = if (isSystemInDarkTheme()) Dark else Light
    val base = if (isSystemInDarkTheme()) darkColorScheme() else lightColorScheme()
    val scheme = base.copy(
        primary = t.accent, onPrimary = t.accentText,
        secondaryContainer = t.surface2, onSecondaryContainer = t.text,
        background = t.bg, onBackground = t.text,
        surface = t.surface, onSurface = t.text,
        surfaceVariant = t.surface2, onSurfaceVariant = t.text2,
        surfaceContainer = t.surface, surfaceContainerLow = t.surface, surfaceContainerHigh = t.surface,
        surfaceContainerHighest = t.surface2,
        outline = t.border, outlineVariant = t.border,
        error = t.danger, onError = Color.White,
    )
    androidx.compose.runtime.CompositionLocalProvider(LocalTokens provides t) {
        MaterialTheme(colorScheme = scheme, typography = type, content = content)
    }
}
