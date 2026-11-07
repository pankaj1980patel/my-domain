package com.mydomain.android.ui

import androidx.annotation.DrawableRes
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.buildAnnotatedString
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.withStyle
import androidx.compose.ui.unit.em
import androidx.compose.ui.unit.sp
import com.mydomain.android.R

// The brand mark as the kit shows it: the galaxy logo, the "My-Domain"
// wordmark with "Domain" in the accent colour, and the gold tagline.

/** The galaxy logo for the current theme: the soft-edged dark mark, or on
 *  Cosmic Light the light galaxy (transparent, no frame of its own). */
@DrawableRes
@Composable
fun logoRes(): Int = if (T.isLight) R.drawable.logo_mark_light else R.drawable.logo_mark

@Composable
fun Wordmark(modifier: Modifier = Modifier, fontSize: Int = 30, maxLines: Int = Int.MAX_VALUE) {
    val accent = T.accentFg
    Text(
        buildAnnotatedString {
            append(stringResource(R.string.brand_wordmark_lead))
            withStyle(SpanStyle(color = accent)) { append(stringResource(R.string.brand_wordmark_accent)) }
        },
        modifier = modifier,
        maxLines = maxLines,
        color = T.text,
        style = TextStyle(fontFamily = Inter, fontSize = fontSize.sp, fontWeight = FontWeight.ExtraBold, letterSpacing = (-0.01).em),
    )
}

@Composable
fun Tagline(modifier: Modifier = Modifier) {
    Text(
        stringResource(R.string.brand_tagline).uppercase(),
        modifier = modifier,
        color = T.highlightFg,
        style = MaterialTheme.typography.labelMedium.copy(fontWeight = FontWeight.Bold, letterSpacing = 0.04.em),
    )
}
