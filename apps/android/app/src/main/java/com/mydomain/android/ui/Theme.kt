package com.mydomain.android.ui

import android.app.Activity
import android.content.Context
import android.content.SharedPreferences
import androidx.annotation.StringRes
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Shapes
import androidx.compose.material3.Typography
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.Immutable
import androidx.compose.runtime.SideEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalView
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.Font
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.em
import androidx.compose.ui.unit.sp
import androidx.core.view.WindowCompat
import com.mydomain.android.R

// The same design tokens as the desktop (apps/desktop/ui/src/styles/tokens.css),
// from the owner's brand kit (docs/design/my-domain-brand-theme-kit.html; the
// rules are in docs/design/theme.md). Five themes; Cosmic Blue is the default.

/** The kit's five themes. `key` is what's stored in SharedPreferences and
 *  matches the desktop's data-theme value. */
enum class ThemeId(val key: String, @StringRes val label: Int, @StringRes val desc: Int) {
    CosmicBlue("cosmic-blue", R.string.theme_cosmic_blue, R.string.theme_cosmic_blue_desc),
    Aurora("aurora", R.string.theme_aurora, R.string.theme_aurora_desc),
    Ocean("ocean", R.string.theme_ocean, R.string.theme_ocean_desc),
    Midnight("midnight", R.string.theme_midnight, R.string.theme_midnight_desc),
    Light("light", R.string.theme_light, R.string.theme_light_desc);

    val tokens: Tokens
        get() = when (this) {
            CosmicBlue -> CosmicBlueTokens
            Aurora -> AuroraTokens
            Ocean -> OceanTokens
            Midnight -> MidnightTokens
            Light -> LightTokens
        }

    companion object {
        fun fromKey(key: String?): ThemeId = entries.firstOrNull { it.key == key } ?: CosmicBlue
    }
}

@Immutable
data class Tokens(
    val bg: Color,
    /** Glass panel (translucent over the sky): opaque enough (78%) that
     *  [text2] and [text3] keep 4.5:1 on it over the star field's
     *  brightest point. */
    val surface: Color,
    /** Raised: inputs, incoming bubbles, selected backgrounds. */
    val surface2: Color,
    /** Opaque panel for bars, dialogs and sheets over moving content. */
    val bar: Color,
    val border: Color,
    val borderStrong: Color,
    /** Edges of text fields and an off switch: 3:1 on every surface
     *  (WCAG 1.4.11). Decorative card borders keep [border]. */
    val controlBorder: Color,
    val text: Color,
    val text2: Color,
    val text3: Color,
    /** Primary action colour; buttons draw [gradient] over it. */
    val primary: Color,
    val onPrimary: Color,
    /** Brand gradient stops (fraction to colour), drawn at about 120 degrees. */
    val gradient: List<Pair<Float, Color>>,
    /** Laid over [gradient] on primary buttons only, so the white label
     *  keeps 4.5:1 at the gradient's lightest stop. */
    val primaryScrim: Color,
    val secondaryButton: Color,
    val secondaryButtonBorder: Color,
    /** Interaction: focus, selection, active navigation. */
    val accent: Color,
    val onAccent: Color,
    /** The accent when used as text or an icon on [surface]. */
    val accentFg: Color,
    /** A faint accent wash behind selected items. */
    val accentBg: Color,
    val accentSoft: Color,
    /** Reserved for rare, important moments. */
    val highlight: Color,
    /** The highlight used as text (the brand tagline). */
    val highlightFg: Color,
    val success: Color,
    val successSoft: Color,
    val warning: Color,
    val danger: Color,
    val dangerSoft: Color,
    val toastBg: Color,
    val toastFg: Color,
    /** Draw the star field behind everything. */
    val starField: Boolean,
    /** Tints the star field towards [bg]. */
    val veil: Color,
    /** The soft radial glow at the top right; Transparent for none. */
    val glow: Color,
    /** Status dots glow (off in the quiet theme). */
    val dotGlow: Boolean,
    val isLight: Boolean,
) {
    /** The brand gradient as a brush (diagonal, like the kit's 120deg). */
    val brush: Brush
        get() = Brush.linearGradient(*gradient.toTypedArray(), start = Offset.Zero, end = Offset.Infinite)
}

private val Gold = Color(0xFFFFC857)
private val Success = Color(0xFF34D399)
private val Danger = Color(0xFFFB7185)

/** Cosmic Blue: the brand default. Deep navy, Royal Blue actions, Electric
 *  Cyan for interaction, Cosmic Gold for highlights. */
private val CosmicBlueTokens = Tokens(
    bg = Color(0xFF050B18), surface = Color(0xC70B1630), surface2 = Color(0xDB101D35), bar = Color(0xF00B1630),
    border = Color(0xFF203454), borderStrong = Color(0xFF2A456A), controlBorder = Color(0xFF5D7598),
    text = Color(0xFFF4F8FF), text2 = Color(0xFFC6D5EA), text3 = Color(0xFF94A3B8),
    primary = Color(0xFF1455D9), onPrimary = Color.White,
    gradient = listOf(0f to Color(0xFF1455D9), 0.55f to Color(0xFF22C7F2), 1f to Color(0xFF713BDB)),
    // #22C7F2 alone is 2.0:1 under white.
    primaryScrim = Color(0x66050B18),
    secondaryButton = Color(0xFF152744), secondaryButtonBorder = Color(0xFF2A456A),
    accent = Color(0xFF22C7F2), onAccent = Color(0xFF041321), accentFg = Color(0xFF22C7F2),
    accentBg = Color(0x1F22C7F2), accentSoft = Color(0xFF64BFFF), highlight = Gold, highlightFg = Gold,
    success = Success, successSoft = Color(0x2434D399), warning = Gold,
    danger = Danger, dangerSoft = Color(0x21FB7185),
    toastBg = Color(0xFFEAF7FF), toastFg = Color(0xFF071525),
    starField = true, veil = Color(0x59050B18), glow = Color(0x55123E79), dotGlow = true, isLight = false,
)

/** Aurora Purple: expressive violet, Aurora Cyan interaction, pink highlights. */
private val AuroraTokens = Tokens(
    bg = Color(0xFF090817), surface = Color(0xC717152E), surface2 = Color(0xDB201D3D), bar = Color(0xF017152E),
    border = Color(0xFF2E2A52), borderStrong = Color(0xFF433C74), controlBorder = Color(0xFF6F679E),
    text = Color(0xFFF7F4FF), text2 = Color(0xFFD6D0EA), text3 = Color(0xFFA8A2C2),
    primary = Color(0xFF713BDB), onPrimary = Color.White,
    gradient = listOf(0f to Color(0xFF312E81), 0.5f to Color(0xFF713BDB), 1f to Color(0xFFE879F9)),
    // #E879F9 alone is 2.5:1 under white.
    primaryScrim = Color(0x52050B18),
    secondaryButton = Color(0xFF221F42), secondaryButtonBorder = Color(0xFF433C74),
    accent = Color(0xFF42D9F5), onAccent = Color(0xFF041321), accentFg = Color(0xFF42D9F5),
    accentBg = Color(0x1F42D9F5), accentSoft = Color(0xFF9B75FF), highlight = Color(0xFFE879F9), highlightFg = Color(0xFFE879F9),
    success = Success, successSoft = Color(0x2434D399), warning = Gold,
    danger = Danger, dangerSoft = Color(0x21FB7185),
    toastBg = Color(0xFFF7F4FF), toastFg = Color(0xFF120F26),
    starField = true, veil = Color(0x66090817), glow = Color(0x4D713BDB), dotGlow = true, isLight = false,
)

/** Ocean Blue: calm blues, Aqua interaction, Mint for "connected". */
private val OceanTokens = Tokens(
    bg = Color(0xFF061321), surface = Color(0xC70D2439), surface2 = Color(0xDB12304A), bar = Color(0xF00D2439),
    border = Color(0xFF1D3D5A), borderStrong = Color(0xFF2A5578), controlBorder = Color(0xFF4F7A9C),
    text = Color(0xFFF2FAFF), text2 = Color(0xFFC4DBE8), text3 = Color(0xFF98B4C6),
    primary = Color(0xFF0759A6), onPrimary = Color.White,
    gradient = listOf(0f to Color(0xFF0759A6), 0.6f to Color(0xFF0A8FC4), 1f to Color(0xFF00B8D9)),
    // #00B8D9 alone is 2.4:1 under white.
    primaryScrim = Color(0x57050B18),
    secondaryButton = Color(0xFF11304B), secondaryButtonBorder = Color(0xFF2A5578),
    accent = Color(0xFF00B8D9), onAccent = Color(0xFF03121F), accentFg = Color(0xFF00B8D9),
    accentBg = Color(0x1F00B8D9), accentSoft = Color(0xFFA5E8FF), highlight = Gold, highlightFg = Gold,
    success = Color(0xFF55E6C1), successSoft = Color(0x2455E6C1), warning = Gold,
    danger = Danger, dangerSoft = Color(0x21FB7185),
    toastBg = Color(0xFFF2FAFF), toastFg = Color(0xFF061321),
    starField = true, veil = Color(0x66061321), glow = Color(0x3300B8D9), dotGlow = true, isLight = false,
)

/** Minimal Midnight: quiet and professional. Solid panels, no star field,
 *  no glow, flat primary buttons. */
private val MidnightTokens = Tokens(
    bg = Color(0xFF080B12), surface = Color(0xFF111827), surface2 = Color(0xFF1C2738), bar = Color(0xFF111827),
    border = Color(0xFF29364A), borderStrong = Color(0xFF36465F), controlBorder = Color(0xFF64758F),
    text = Color(0xFFF8FAFC), text2 = Color(0xFFCBD5E1), text3 = Color(0xFF94A3B8),
    primary = Color(0xFF2563EB), onPrimary = Color.White,
    gradient = listOf(0f to Color(0xFF2563EB), 1f to Color(0xFF2563EB)),
    // Flat #2563EB is 5.2:1 already.
    primaryScrim = Color.Transparent,
    secondaryButton = Color(0xFF1C2738), secondaryButtonBorder = Color(0xFF36465F),
    accent = Color(0xFF38BDF8), onAccent = Color(0xFF04131F), accentFg = Color(0xFF38BDF8),
    accentBg = Color(0x1A38BDF8), accentSoft = Color(0xFF38BDF8), highlight = Gold, highlightFg = Gold,
    success = Success, successSoft = Color(0x1F34D399), warning = Gold,
    danger = Danger, dangerSoft = Color(0x1FFB7185),
    toastBg = Color(0xFFF8FAFC), toastFg = Color(0xFF080B12),
    starField = false, veil = Color.Transparent, glow = Color.Transparent, dotGlow = false, isLight = false,
)

/** Cosmic Light: the light-mode companion. Cloud background, white cards,
 *  Ink text. */
private val LightTokens = Tokens(
    bg = Color(0xFFF4F8FF), surface = Color(0xDBFFFFFF), surface2 = Color(0xFFE7F0FF), bar = Color(0xF5FFFFFF),
    border = Color(0xFFDCE8F7), borderStrong = Color(0xFFC3D5EC), controlBorder = Color(0xFF73859E),
    // Muted Blue deepened to 4.5:1 on surface2 (was #61738D).
    text = Color(0xFF10213D), text2 = Color(0xFF33476A), text3 = Color(0xFF56677F),
    primary = Color(0xFF1769E8), onPrimary = Color.White,
    gradient = listOf(0f to Color(0xFF1769E8), 0.55f to Color(0xFF38BDF8), 1f to Color(0xFF713BDB)),
    // #38BDF8 alone is 2.1:1 under white.
    primaryScrim = Color(0x61050B18),
    secondaryButton = Color.White, secondaryButtonBorder = Color(0xFFC3D5EC),
    // Accent was #38BDF8 (2.1:1 on white): switches, the battery line and
    // selection rings need 3:1. Status colours and the tagline darkened to
    // 4.5:1 on surface2 (were #059669, #B45309, #E11D48, #9A6400).
    accent = Color(0xFF0284C7), onAccent = Color(0xFF041321), accentFg = Color(0xFF0369A1),
    accentBg = Color(0x171769E8), accentSoft = Color(0xFF38BDF8), highlight = Color(0xFFF5B942), highlightFg = Color(0xFF8A5900),
    success = Color(0xFF047857), successSoft = Color(0x1F059669), warning = Color(0xFF9A4508),
    danger = Color(0xFFBE123C), dangerSoft = Color(0x1AE11D48),
    toastBg = Color(0xFF10213D), toastFg = Color(0xFFF4F8FF),
    starField = false, veil = Color.Transparent, glow = Color(0x3838BDF8), dotGlow = true, isLight = true,
)

/** The chosen theme, persisted in the app's "md" SharedPreferences under
 *  "theme". Compose re-themes the whole app when [current] changes. */
object ThemeStore {
    const val PREF_KEY = "theme"
    var current by mutableStateOf(ThemeId.CosmicBlue)
        private set
    private var loaded = false

    fun load(prefs: SharedPreferences) {
        if (loaded) return
        loaded = true
        current = ThemeId.fromKey(prefs.getString(PREF_KEY, null))
    }

    /** Debug builds' screenshots (`--es md_theme light|dark`): this theme
     *  for now, not saved. */
    fun preview(id: ThemeId) {
        loaded = true
        current = id
    }

    /** Switch now and remember it for the next launch. */
    fun set(prefs: SharedPreferences, id: ThemeId) {
        current = id
        prefs.edit().putString(PREF_KEY, id.key).apply()
    }
}

val LocalTokens = staticCompositionLocalOf { CosmicBlueTokens }

/** The current design tokens: `T.text`, `T.border`, ... */
val T: Tokens
    @Composable get() = LocalTokens.current

/** Inter, bundled in res/font (OFL; licence in assets/licenses/Inter-OFL.txt,
 *  from the npm package inter-font 3.19.0). Works offline. */
val Inter = FontFamily(
    Font(R.font.inter_regular, FontWeight.Normal),
    Font(R.font.inter_medium, FontWeight.Medium),
    Font(R.font.inter_semibold, FontWeight.SemiBold),
    Font(R.font.inter_bold, FontWeight.Bold),
    Font(R.font.inter_extrabold, FontWeight.ExtraBold),
)

/** Kit type: Inter, 15 sp body at 1.55 line height, titles in extra-bold. */
private val type = Typography().let { base ->
    Typography(
        displayLarge = base.displayLarge.copy(fontFamily = Inter),
        displayMedium = base.displayMedium.copy(fontFamily = Inter),
        displaySmall = base.displaySmall.copy(fontFamily = Inter),
        headlineLarge = base.headlineLarge.copy(fontFamily = Inter),
        headlineMedium = base.headlineMedium.copy(fontFamily = Inter),
        headlineSmall = TextStyle(fontFamily = Inter, fontSize = 24.sp, lineHeight = 28.sp, fontWeight = FontWeight.ExtraBold, letterSpacing = (-0.01).em),
        titleLarge = TextStyle(fontFamily = Inter, fontSize = 18.sp, fontWeight = FontWeight.Bold),
        titleMedium = TextStyle(fontFamily = Inter, fontSize = 15.sp, fontWeight = FontWeight.Bold),
        titleSmall = base.titleSmall.copy(fontFamily = Inter),
        bodyLarge = TextStyle(fontFamily = Inter, fontSize = 15.sp, lineHeight = 23.sp),
        bodyMedium = TextStyle(fontFamily = Inter, fontSize = 14.sp, lineHeight = 21.sp),
        bodySmall = TextStyle(fontFamily = Inter, fontSize = 13.sp, lineHeight = 19.sp),
        labelLarge = TextStyle(fontFamily = Inter, fontSize = 14.sp, fontWeight = FontWeight.Bold),
        labelMedium = TextStyle(fontFamily = Inter, fontSize = 12.sp, fontWeight = FontWeight.SemiBold),
        // The kit's "eyebrow": small spaced capitals. Nothing in the app
        // is smaller than 12 sp.
        labelSmall = TextStyle(fontFamily = Inter, fontSize = 12.sp, fontWeight = FontWeight.ExtraBold, letterSpacing = 0.12.em),
    )
}

/** Kit radii: buttons and inputs 12, cards 18, hero surfaces 28. */
private val shapes = Shapes(
    extraSmall = RoundedCornerShape(8.dp),
    small = RoundedCornerShape(12.dp),
    medium = RoundedCornerShape(20.dp),
    large = RoundedCornerShape(22.dp),
    extraLarge = RoundedCornerShape(28.dp),
)

@Composable
fun MdTheme(content: @Composable () -> Unit) {
    val ctx = LocalContext.current
    remember { ThemeStore.load(ctx.getSharedPreferences("md", Context.MODE_PRIVATE)) }
    val t = ThemeStore.current.tokens
    val base = if (t.isLight) lightColorScheme() else darkColorScheme()
    val scheme = base.copy(
        primary = t.primary, onPrimary = t.onPrimary,
        primaryContainer = t.accentBg, onPrimaryContainer = t.accentFg,
        secondary = t.accent, onSecondary = t.onAccent,
        secondaryContainer = t.accentBg, onSecondaryContainer = t.text,
        tertiary = t.highlight,
        background = Color.Transparent, onBackground = t.text,
        surface = t.surface, onSurface = t.text,
        surfaceVariant = t.surface2, onSurfaceVariant = t.text2,
        surfaceContainerLowest = t.bar, surfaceContainerLow = t.bar, surfaceContainer = t.bar,
        surfaceContainerHigh = t.bar, surfaceContainerHighest = t.surface2,
        inverseSurface = t.toastBg, inverseOnSurface = t.toastFg, inversePrimary = t.accent,
        // Material draws component edges in `outline`: the 3:1 control edge.
        outline = t.controlBorder, outlineVariant = t.border,
        error = t.danger, onError = Color.White, errorContainer = t.dangerSoft, onErrorContainer = t.danger,
        scrim = Color(0x99000000),
    )

    // Status and navigation bar icons: dark on the light theme, light otherwise.
    val view = LocalView.current
    if (!view.isInEditMode) {
        SideEffect {
            val window = (view.context as? Activity)?.window ?: return@SideEffect
            WindowCompat.getInsetsController(window, view).apply {
                isAppearanceLightStatusBars = t.isLight
                isAppearanceLightNavigationBars = t.isLight
            }
        }
    }

    CompositionLocalProvider(LocalTokens provides t) {
        MaterialTheme(colorScheme = scheme, typography = type, shapes = shapes) {
            Box(Modifier.fillMaxSize().background(t.bg)) {
                if (t.starField) {
                    Image(
                        painterResource(R.drawable.space_bg), contentDescription = null,
                        modifier = Modifier.fillMaxSize(), contentScale = ContentScale.Crop,
                    )
                    Box(Modifier.fillMaxSize().background(t.veil))
                }
                if (t.glow != Color.Transparent) {
                    Box(
                        Modifier.fillMaxSize().drawBehind {
                            drawRect(
                                Brush.radialGradient(
                                    listOf(t.glow, Color.Transparent),
                                    center = Offset(size.width * 0.75f, -size.height * 0.15f),
                                    radius = size.maxDimension * 0.6f,
                                ),
                            )
                        },
                    )
                }
                content()
            }
        }
    }
}
