package com.mydomain.android.ui

import androidx.compose.animation.core.RepeatMode
import androidx.compose.animation.core.animateFloat
import androidx.compose.animation.core.infiniteRepeatable
import androidx.compose.animation.core.rememberInfiniteTransition
import androidx.compose.animation.core.tween
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import com.mydomain.android.R

/** A grey bar standing in for a line of text while it loads. */
@Composable
private fun SkeletonBar(width: Float, height: Dp, alpha: Float) {
    Box(Modifier.fillMaxWidth(width).height(height).alpha(alpha).background(T.surface2, RoundedCornerShape(6.dp)))
}

/** Placeholder cards while the device list is read the first time:
 *  [rows] device rows, after a big card when [hero] (as Home has). One
 *  TalkBack stop, "Loading your devices…"; no pulse with animations off. */
@Composable
fun DeviceSkeleton(rows: Int, hero: Boolean = false) {
    val loading = stringResource(R.string.devices_loading)
    val pulse = if (reducedMotion()) {
        1f
    } else {
        val tr = rememberInfiniteTransition(label = "skeleton")
        val a by tr.animateFloat(0.45f, 0.9f, infiniteRepeatable(tween(900), RepeatMode.Reverse), label = "a")
        a
    }
    Column(
        Modifier.fillMaxWidth().clearAndSetSemantics { contentDescription = loading },
        verticalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        if (hero) {
            SoftCard {
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Box(Modifier.size(96.dp).alpha(pulse).background(T.surface2, RoundedCornerShape(20.dp)))
                    Spacer(Modifier.width(14.dp))
                    Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(10.dp)) {
                        SkeletonBar(0.7f, 20.dp, pulse)
                        SkeletonBar(0.45f, 14.dp, pulse)
                        SkeletonBar(0.6f, 14.dp, pulse)
                    }
                }
            }
        }
        SoftCard(padding = PaddingValues(12.dp)) {
            repeat(rows) {
                Row(Modifier.heightIn(min = 56.dp), verticalAlignment = Alignment.CenterVertically) {
                    Box(Modifier.size(48.dp).alpha(pulse).background(T.surface2, CircleShape))
                    Spacer(Modifier.width(12.dp))
                    Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                        SkeletonBar(0.55f, 14.dp, pulse)
                        SkeletonBar(0.35f, 12.dp, pulse)
                    }
                }
            }
        }
    }
}
