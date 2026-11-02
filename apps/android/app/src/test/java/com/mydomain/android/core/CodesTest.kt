package com.mydomain.android.core

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class CodesTest {

    private fun code(text: String) = findCode(text)

    @Test
    fun typicalOtpMessages() {
        assertEquals("482913", code("482913 is your OTP for login to HDFC Bank NetBanking. Do not share it with anyone."))
        assertEquals("123456", code("Your OTP for login is 123456. Valid for 10 min. Do not share. -SBI"))
        assertEquals("4567", code("Your Swiggy verification code is 4567"))
        assertEquals("839201", code("Use 839201 as your one-time password"))
        assertEquals("771204", code("Dear Customer, use One Time Password 771204 to complete your Paytm login."))
        assertEquals("5521", code("Your Zomato passcode: 5521"))
        assertEquals("90817263", code("Use PIN 90817263 to verify your account"))
        assertEquals("246810", code("Amazon: 246810 is your Amazon OTP. Don't share it with anyone."))
    }

    @Test
    fun googleStylePrefix() {
        assertEquals("123456", code("G-123456 is your Google verification code."))
    }

    @Test
    fun spacedOrDashedCode() {
        assertEquals("123456", code("Your WhatsApp code: 123-456"))
        assertEquals("987654", code("Your verification code is 987 654"))
    }

    @Test
    fun alphanumericCode() {
        assertEquals("AB12CD34", code("Your verification code is AB12CD34"))
    }

    @Test
    fun hindi() {
        assertEquals("567890", code("आपका ओटीपी 567890 है। इसे किसी के साथ साझा न करें।"))
        assertEquals("4412", code("आपका सत्यापन कोड 4412 है"))
    }

    @Test
    fun skipsAmountsAndCards() {
        assertEquals(
            "4321",
            code("Dear customer, 4321 is the OTP for your transaction of Rs. 1,500.00 at Amazon on card XX1234."),
        )
        assertEquals("482913", code("OTP for txn of INR 25000 on your HDFC card ending 4455 is 482913."))
        assertEquals("662211", code("Pay ₹2499 using OTP 662211"))
        assertEquals("310572", code("OTP 310572 for payment of $1500. Valid till 10:30"))
    }

    @Test
    fun skipsPhoneNumbersAndDates() {
        assertEquals("1928", code("Your code is 1928. Call 9876543210 if this wasn't you. 02-11-2026"))
        assertEquals("5038", code("Call +91 98765 43210 for help. Your OTP is 5038"))
    }

    @Test
    fun yearOnlyIfNothingElse() {
        assertEquals("7812", code("Verification code for Nov 2026: 7812"))
    }

    @Test
    fun prefersNearestToKeyword() {
        assertEquals(
            "550123",
            code("Parcel 84512 arriving today. Delivery OTP 550123, share with the delivery partner."),
        )
    }

    @Test
    fun negatives() {
        assertNull(code("Your order #12345678 has shipped"))
        assertNull(code("Rs 5000 credited to a/c XX1234 on 02-11-2026"))
        assertNull(code("Use code SAVE20 for 20% off"))
        assertNull(code("Use code DIWALI2026 to save"))
        assertNull(code("Call 9876543210 to reset your PIN"))
        assertNull(code("Rs.15000 debited from A/c XX7788. Never share your UPI PIN. Ref 612345678901"))
        assertNull(code("Meeting moved to 4:30, see you there"))
        assertNull(code("Hey, are we still on for 1830 tonight?"))
        assertNull(code("Your OTP has expired. Request a new one."))
        assertNull(code(""))
    }

    @Test
    fun everyResultPassesIsCode() {
        listOf("G-123456 is your Google verification code.", "Your code: AB12CD34", "OTP 1234")
            .mapNotNull(::findCode)
            .forEach { assertTrue(it, isCode(it)) }
    }
}
