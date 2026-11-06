package com.mydomain.android.core

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/** Verification codes as notifications show them (messaging, bank and
 *  sign-in apps), in English and Hindi: the code, how sure, and who from. */
class OtpDetectTest {

    /** [code] null: nothing should be sent. [issuer] null: not checked. */
    private data class Sample(
        val text: String,
        val code: String?,
        val issuer: String? = null,
        val title: String = "",
        val app: String = "Messages",
    )

    private val samples = listOf(
        // Plain English, all sorts of senders.
        Sample("482913 is your OTP for login to HDFC Bank NetBanking. Do not share it with anyone.", "482913", "HDFC Bank"),
        Sample("Your OTP for login is 123456. Valid for 10 min. Do not share. -SBI", "123456", "SBI"),
        Sample("Your Swiggy verification code is 4567", "4567", "Swiggy"),
        Sample("G-123456 is your Google verification code.", "123456", "Google"),
        Sample("Amazon: 246810 is your Amazon OTP. Don't share it with anyone.", "246810", "Amazon"),
        Sample("[Microsoft] Use 70412983 as your security code", "70412983", "Microsoft"),
        Sample("Your WhatsApp code: 123-456. Don't share this code with others", "123456", "WhatsApp"),
        Sample("Your verification code is 987 654", "987654"),
        Sample("Dear Customer, use One Time Password 771204 to complete your Paytm login.", "771204", "Paytm"),
        Sample("Your Uber code: 5521. Never share this code.", "5521", "Uber"),
        Sample("Use PIN 90817263 to verify your account", "90817263"),
        Sample("Welcome to Zepto! Your login code is 3391", "3391", "Zepto"),
        Sample("Sign-in code from Apple: 615 204", "615204", "Apple"),
        Sample("Your verification code is AB12CD34", "AB12CD34"),
        Sample("Your one-time passcode is K7XQ2P9Z. It expires in 10 minutes.", "K7XQ2P9Z"),
        Sample("Your Netflix sign-in code is 8812. If you didn't request it, ignore this.", "8812", "Netflix"),
        // The title is the sender in a messaging app.
        Sample("582914 is your OTP for login. Do not share it with anyone.", "582914", "HDFCBK", title = "VM-HDFCBK"),
        Sample("Your code is 302118", "302118", "Flipkart", title = "Flipkart"),
        // A bank's own app: the title says it.
        Sample("Use 449012 to approve the payment. Valid for 3 minutes.", "449012", title = "OTP for transaction", app = "ICICI iMobile"),
        // Hindi.
        Sample("आपका ओटीपी 567890 है। इसे किसी के साथ साझा न करें।", "567890"),
        Sample("आपका सत्यापन कोड 4412 है", "4412"),
        Sample("एसबीआई का ओटीपी 778899 है। कृपया किसी को न बताएं।", "778899", "एसबीआई"),
        Sample("लॉगिन के लिए ओटीपी: 120934. यह 5 मिनट के लिए मान्य है।", "120934"),
        Sample("Paytm पर लॉगिन के लिए आपका ओटीपी 664120 है", "664120"),
        // Amounts beside the code.
        Sample("Dear customer, 4321 is the OTP for your transaction of Rs. 1,500.00 at Amazon on card XX1234.", "4321"),
        Sample("OTP for txn of INR 25000 on your HDFC card ending 4455 is 482913.", "482913"),
        Sample("Pay ₹2499 using OTP 662211", "662211"),
        Sample("OTP 310572 for payment of $1500. Valid till 10:30", "310572"),
        Sample("₹ 12500 will be debited. OTP is 9087. Do not share.", "9087"),
        Sample("आपके खाते से रु. 5000 के लेनदेन के लिए ओटीपी 553201 है", "553201"),
        // Phone numbers, dates, times, years, labelled numbers beside it.
        Sample("Your code is 1928. Call 9876543210 if this wasn't you. 02-11-2026", "1928"),
        Sample("Call +91 98765 43210 for help. Your OTP is 5038", "5038"),
        Sample("Verification code for Nov 2026: 7812", "7812"),
        Sample("Order #4455667 placed. Delivery OTP 2468, share with the delivery partner.", "2468"),
        Sample("Parcel 84512 arriving today. Delivery OTP 550123, share with the delivery partner.", "550123"),
        Sample("On 12/11/2026 at 14:05 your OTP is 330021", "330021"),
        // Nothing to send.
        Sample("Your order #12345678 has shipped", null),
        Sample("Rs 5000 credited to a/c XX1234 on 02-11-2026", null),
        Sample("Use code SAVE20 for 20% off", null),
        Sample("Use promo code FEST2026 to get flat 100 off", null),
        Sample("Call 9876543210 to reset your PIN", null),
        Sample("Rs.15000 debited from A/c XX7788. Never share your UPI PIN. Ref 612345678901", null),
        Sample("Meeting moved to 4:30, see you there", null),
        Sample("Hey, are we still on for 1830 tonight?", null),
        Sample("Your OTP has expired. Request a new one.", null),
        Sample("Your verification code will arrive shortly", null),
        Sample("Order ID 99887766 confirmed. Use code to track.", null),
        Sample("आपके खाते में रु. 2500 जमा हुए", null),
    )

    @Test
    fun everySampleFindsTheRightCode() {
        val wrong = samples.mapNotNull { s ->
            val got = detectCode(s.text, s.title, s.app)
            val sent = got?.takeIf { it.confidence >= MIN_CONFIDENCE }?.code
            if (sent != s.code) "${s.text} -> $got (want ${s.code})" else null
        }
        assertTrue(wrong.joinToString("\n"), wrong.isEmpty())
        assertTrue("about forty samples", samples.size >= 40)
    }

    @Test
    fun theIssuerIsWhoTheTextSaysItsFrom() {
        val wrong = samples.filter { it.issuer != null }.mapNotNull { s ->
            val got = detectCode(s.text, s.title, s.app)?.issuer
            if (got != s.issuer) "${s.text} -> issuer $got (want ${s.issuer})" else null
        }
        assertTrue(wrong.joinToString("\n"), wrong.isEmpty())
    }

    @Test
    fun aCodeRightByTheWordIsSurerThanOneFarAway() {
        val near = detectCode("Your OTP is 482913")!!
        val far = detectCode("482913 — we sent this because someone asked to reset the code on your account")!!
        assertTrue("$near vs $far", near.confidence > far.confidence)
        assertEquals(100, detectCode("Your OTP is 482913. Do not share it.")!!.confidence)
    }

    @Test
    fun onlyCodeIsWeakerThanOtp() {
        val weak = detectCode("Your code is 482913")!!
        val strong = detectCode("Your OTP is 482913")!!
        assertTrue(weak.confidence < strong.confidence)
        assertTrue(weak.confidence >= MIN_CONFIDENCE)
    }

    @Test
    fun theAppIsNeverItsOwnIssuer() {
        assertEquals("", detectCode("Your code is 302118", title = "Messages", app = "Messages")!!.issuer)
        assertEquals("", issuerOf("Your OTP is 123456", title = "+91 98765 43210"))
    }

    @Test
    fun targetsAreTheChosenComputersThatTakeCodes() {
        val pcs = listOf("mac" to true, "win" to true, "old" to false)
        assertEquals(listOf("mac", "win"), codeTargets(pcs, null))
        assertEquals(listOf("win"), codeTargets(pcs, setOf("win", "old")))
        assertEquals(emptyList<String>(), codeTargets(pcs, emptySet()))
        assertNull(detectCode(""))
    }
}
