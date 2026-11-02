package com.mydomain.android.ui

import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.ui.Modifier
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.res.painterResource
import com.mydomain.android.R
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Typography
import androidx.compose.material3.darkColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.Immutable
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.sp

// The same design tokens as the desktop (apps/desktop/ui/src/styles/tokens.css):
// a deep-space theme, always dark.

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

/** Deep space: a star field behind everything (drawable/space_bg, cropped from
 *  docs/brand/space-source.png), translucent glass panels, white primary actions. */
private val Space = Tokens(
    bg = Color(0xFF02080F), surface = Color(0x14FFFFFF), surface2 = Color(0x1FFFFFFF),
    border = Color(0x1FFFFFFF), text = Color(0xFFF4F4F6), text2 = Color(0xFFB6B8C6), text3 = Color(0xFF8D90A3),
    accent = Color.White, accentText = Color(0xFF0A0D1A),
    success = Color(0xFF4ADE80), successSoft = Color(0x244ADE80), warning = Color(0xFFFACC15),
    danger = Color(0xFFF87171), dangerSoft = Color(0x21F87171),
)

/** Solid panel colour for bars and sheets that sit over moving content. */
val SpaceBar = Color(0xE60D1120)

val LocalTokens = staticCompositionLocalOf { Space }

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
    val t = Space
    val scheme = darkColorScheme().copy(
        primary = t.accent, onPrimary = t.accentText,
        secondaryContainer = t.surface2, onSecondaryContainer = t.text,
        background = Color.Transparent, onBackground = t.text,
        surface = t.surface, onSurface = t.text,
        surfaceVariant = t.surface2, onSurfaceVariant = t.text2,
        surfaceContainer = SpaceBar, surfaceContainerLow = SpaceBar, surfaceContainerHigh = SpaceBar,
        surfaceContainerHighest = t.surface2, inverseSurface = t.text, inverseOnSurface = t.accentText,
        outline = t.border, outlineVariant = t.border,
        error = t.danger, onError = Color.White,
    )
    androidx.compose.runtime.CompositionLocalProvider(LocalTokens provides t) {
        MaterialTheme(colorScheme = scheme, typography = type) {
            Box(Modifier.fillMaxSize().background(t.bg)) {
                Image(
                    painterResource(R.drawable.space_bg), contentDescription = null,
                    modifier = Modifier.fillMaxSize(), contentScale = ContentScale.Crop,
                )
                content()
            }
        }
    }
}
