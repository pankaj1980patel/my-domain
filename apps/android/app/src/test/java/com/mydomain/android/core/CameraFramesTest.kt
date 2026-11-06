package com.mydomain.android.core

import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test
import java.nio.ByteBuffer
import java.util.TimeZone

class CameraFramesTest {

    @Test
    fun theLargestPreviewThatFitsIsPicked() {
        val sizes = listOf(4000 to 3000, 1920 to 1080, 1280 to 720, 1280 to 960, 640 to 480, 320 to 240)
        assertEquals(1280 to 960, CameraFrames.pickSize(sizes))
        assertEquals(640 to 480, CameraFrames.pickSize(sizes, maxLong = 800))
        // Nothing that small: the smallest there is.
        assertEquals(1920 to 1080, CameraFrames.pickSize(listOf(4000 to 3000, 1920 to 1080)))
        assertNull(CameraFrames.pickSize(emptyList()))
        assertNull(CameraFrames.pickSize(listOf(0 to 0)))
    }

    @Test
    fun theBackCameraTurnsWithThePhoneAndTheFrontOneTheOtherWay() {
        // Most back cameras sit at 90, front ones at 270.
        assertEquals(90, CameraFrames.rotation(90, 0, front = false))
        assertEquals(180, CameraFrames.rotation(90, 90, front = false))
        assertEquals(0, CameraFrames.rotation(90, 270, front = false))
        assertEquals(270, CameraFrames.rotation(270, 0, front = true))
        assertEquals(180, CameraFrames.rotation(270, 90, front = true))
        assertEquals(0, CameraFrames.rotation(270, 270, front = true))
        // Odd inputs still land on a quarter turn.
        assertEquals(90, CameraFrames.rotation(450, -360, front = false))
    }

    @Test
    fun howThePhoneIsHeldSnapsToQuarterTurnsWithSomeSlack() {
        assertEquals(0, CameraFrames.quarterTurn(10, 0))
        assertEquals(90, CameraFrames.quarterTurn(100, 0))
        assertEquals(0, CameraFrames.quarterTurn(350, 0))
        assertEquals(270, CameraFrames.quarterTurn(260, 0))
        // Just past the line between two: not yet.
        assertEquals(0, CameraFrames.quarterTurn(50, 0))
        assertEquals(90, CameraFrames.quarterTurn(50, 90))
        // Flat on a desk: as it was.
        assertEquals(180, CameraFrames.quarterTurn(-1, 180))
    }

    @Test
    fun aTurnedPictureSwapsItsSides() {
        assertEquals(720 to 1280, CameraFrames.rotatedSize(1280, 720, 90))
        assertEquals(1280 to 720, CameraFrames.rotatedSize(1280, 720, 180))
        assertEquals(720 to 1280, CameraFrames.rotatedSize(1280, 720, 270))
    }

    /** A 4 x 2 NV21 picture: Y 0..7 row by row, then one row of V,U pairs. */
    private val pic = byteArrayOf(0, 1, 2, 3, 4, 5, 6, 7, 100, 101, 110, 111)

    @Test
    fun nv21TurnsAQuarterAtATime() {
        // 90 clockwise: 2 wide, 4 tall; the bottom-left corner comes to the top left.
        assertArrayEquals(
            byteArrayOf(4, 0, 5, 1, 6, 2, 7, 3, 100, 101, 110, 111),
            CameraFrames.rotateNv21(pic, 4, 2, 90),
        )
        assertArrayEquals(
            byteArrayOf(7, 6, 5, 4, 3, 2, 1, 0, 110, 111, 100, 101),
            CameraFrames.rotateNv21(pic, 4, 2, 180),
        )
        assertArrayEquals(
            byteArrayOf(3, 7, 2, 6, 1, 5, 0, 4, 110, 111, 100, 101),
            CameraFrames.rotateNv21(pic, 4, 2, 270),
        )
        assertArrayEquals(pic, CameraFrames.rotateNv21(pic, 4, 2, 0))
        // Four quarter turns are none.
        var p = pic
        var (w, h) = 4 to 2
        repeat(4) {
            p = CameraFrames.rotateNv21(p, w, h, 90)
            val s = CameraFrames.rotatedSize(w, h, 90)
            w = s.first
            h = s.second
        }
        assertArrayEquals(pic, p)
    }

    @Test
    fun yuvPlanesBecomeNv21WhateverTheirStrides() {
        // 4 x 2, Y rows padded to 6, U and V interleaved (pixel stride 2)
        // in one row padded to 6, as many phones give them.
        val y = ByteBuffer.wrap(byteArrayOf(0, 1, 2, 3, 9, 9, 4, 5, 6, 7))
        val uv = byteArrayOf(50, 100, 51, 101)
        val u = ByteBuffer.wrap(uv)
        val v = ByteBuffer.wrap(byteArrayOf(100, 50, 101, 51))
        assertArrayEquals(
            byteArrayOf(0, 1, 2, 3, 4, 5, 6, 7, 100, 50, 101, 51),
            CameraFrames.toNv21(4, 2, y, 6, u, v, 6, 2),
        )
        // Planar U and V (pixel stride 1).
        val u1 = ByteBuffer.wrap(byteArrayOf(50, 51))
        val v1 = ByteBuffer.wrap(byteArrayOf(100, 101))
        val y1 = ByteBuffer.wrap(byteArrayOf(0, 1, 2, 3, 4, 5, 6, 7))
        assertArrayEquals(
            byteArrayOf(0, 1, 2, 3, 4, 5, 6, 7, 100, 50, 101, 51),
            CameraFrames.toNv21(4, 2, y1, 4, u1, v1, 2, 1),
        )
    }

    @Test
    fun aPhotoIsNamedForTheComputerToKnowIt() {
        val was = TimeZone.getDefault()
        try {
            TimeZone.setDefault(TimeZone.getTimeZone("UTC"))
            assertEquals("camera-20261106-143005.jpg", CameraFrames.photoName(1_793_975_405_000L))
        } finally {
            TimeZone.setDefault(was)
        }
    }
}
