package com.mydomain.android.core

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class ScreenTouchTest {

    private fun near(want: Pair<Float, Float>, got: Pair<Float, Float>?) {
        requireNotNull(got)
        assertEquals(want.first, got.first, 1f)
        assertEquals(want.second, got.second, 1f)
    }

    @Test
    fun sameShapeMapsStraight() {
        // A 720 x 1600 picture of a 1080 x 2400 screen.
        near(540f to 1200f, ScreenTouch.toScreen(5000, 5000, 720, 1600, 1080, 2400))
        near(0f to 0f, ScreenTouch.toScreen(0, 0, 720, 1600, 1080, 2400))
        near(1079f to 2399f, ScreenTouch.toScreen(10_000, 10_000, 720, 1600, 1080, 2400))
    }

    @Test
    fun aTurnedPhoneHasBarsAboveAndBelow() {
        // Portrait picture, landscape screen: the screen fills the middle
        // 720 x 324 of the 720 x 1600 picture.
        near(1200f to 540f, ScreenTouch.toScreen(5000, 5000, 720, 1600, 2400, 1080))
        // On the bar above: nothing there.
        assertNull(ScreenTouch.toScreen(5000, 500, 720, 1600, 2400, 1080))
        // Top left of the screen part.
        val top = (1600 - 324) / 2f / 1600 * 10_000
        near(0f to 0f, ScreenTouch.toScreen(0, top.toInt() + 1, 720, 1600, 2400, 1080))
    }

    @Test
    fun spotsOffThePictureStayOnIt() {
        near(1079f to 2399f, ScreenTouch.toScreen(20_000, 99_999, 720, 1600, 1080, 2400))
        assertNull(ScreenTouch.toScreen(10, 10, 0, 1600, 1080, 2400))
    }
}
