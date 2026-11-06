package com.mydomain.android.core

/*
 * Finding a one-time verification code in a notification's text, so just
 * the code can be sent to your computers (`MdEngine.sendVerificationCode`).
 * Only notifications are read (the listener that's there already): never
 * SMS, the call log or contacts. Pure Kotlin: unit-tested on the JVM
 * (src/test/.../CodesTest.kt, OtpDetectTest.kt).
 */

/** A code found in a notification: [confidence] 0–100 (send at
 *  [MIN_CONFIDENCE] or more), [issuer] who it's from when the text says
 *  ("Google", "HDFC Bank"), else empty. */
data class CodeMatch(val code: String, val confidence: Int, val issuer: String)

/** Below this a candidate isn't sent. */
const val MIN_CONFIDENCE = 60

/** Words that say "this is a one-time code" outright. */
private val STRONG = Regex(
    "(?i)(?<![\\p{L}\\p{N}])(?:otp|one[- ]?time(?: ?(?:password|passcode|pin|code))?|verification|verify|" +
        "passcode|pass code|security code|login code|log-?in code|sign[- ]?in code|auth(?:entication)? code|" +
        "2fa|two[- ]factor|2-step|confirmation code|access code|" +
        "código de verificación|código de verificação|code de vérification|vérification|" +
        "bestätigungscode|sicherheitscode|codice di verifica|kode verifikasi|kode otp|doğrulama|" +
        "код подтверждения)(?![\\p{L}\\p{N}])" +
        "|ओटीपी|सत्यापन|पासकोड|वन टाइम|वेरिफ़िकेशन|वेरिफिकेशन|验证码|認証コード|確認コード|인증번호|ওটিপি|ஓடிபி|రహస్య",
)

/** Words that say "code" but also mean a coupon or a PIN you already have. */
private val WEAK = Regex(
    "(?i)(?<![\\p{L}\\p{N}])(?:codes?|pin|código|codice|kode|kod|код)(?![\\p{L}\\p{N}])|कोड|पिन|কোড|குறியீடு|رمز",
)

/** Coupon talk: a letters-and-digits "code" here is a promo code. */
private val PROMO = Regex("(?i)\\b(?:coupon|promo|offer|discount|cashback|sale|voucher|referral|% ?off|flat \\d)")

/** Phrases one-time codes come with. */
private val OTP_TALK = Regex("(?i)do not share|don'?t share|never share|valid (?:for|till|until)|expires? in|किसी के साथ|साझा न")

/** Just before a number: money, or a labelled account / card / order /
 *  reference / phone number. */
private val NOT_CODE_BEFORE = Regex(
    "(?i)(?:₹|\\$|€|£|\\brs\\.?|\\binr|\\busd|\\beur|\\bamount|\\bamt\\.?|\\bbal(?:ance)?|\\ba/c|\\bacct|" +
        "\\baccount(?: no\\.?| number)?|\\bcard(?: no\\.?)?|\\bending(?: with| in)?|\\bref(?:erence)?(?: no\\.?| id)?|" +
        "\\btxn(?: id| no\\.?)?|\\btransaction(?: id| no\\.?)?|\\border(?: no\\.?| id| number)?|\\binvoice(?: no\\.?)?|" +
        "\\bawb|\\btracking(?: no\\.?| id| number)?|\\bpnr|\\bbooking(?: id| no\\.?)?|\\bticket(?: no\\.?)?|" +
        "\\bpolicy(?: no\\.?)?|\\bcustomer id|\\bno\\.?|\\bid|\\bcall|\\btel\\.?|\\bphone|\\bmobile|\\bhelpline|" +
        "\\bdial|\\bjan|\\bfeb|\\bmar|\\bapr|\\bmay|\\bjun|\\bjul|\\baug|\\bsep|\\bsept|\\boct|\\bnov|\\bdec|" +
        "\\bjanuary|\\bfebruary|\\bmarch|\\bapril|\\bjune|\\bjuly|\\baugust|\\bseptember|\\boctober|\\bnovember|" +
        "\\bdecember|#|रु\\.?|रुपये)\\s*[:.#-]?\\s*$",
)

/** Just after a number: money, a percentage, a time or a length. */
private val NOT_CODE_AFTER = Regex(
    "(?i)^(?:\\s*(?:rs\\b|inr\\b|rupees|usd\\b|dollars|%|am\\b|pm\\b|hrs?\\b|hours?\\b|mins?\\b|minutes|secs?\\b|" +
        "seconds|days?\\b|gb\\b|mb\\b|kg\\b|km\\b|रुपये)|/-|\\.\\d)",
)

/** A keyword right before the code: "OTP is", "code:", "कोड है". */
private val RIGHT_AFTER_KEYWORD = Regex("(?i)^\\s*(?:is|:|-|=|है|was|reads)?\\s*[:\\-]?\\s*$")

private val RUN = Regex("[A-Za-z0-9]+")

private class Run(val start: Int, val end: Int, val value: String) {
    val len get() = value.length
    val digits = value.all { it in '0'..'9' }
}

private class Cand(val code: String, val start: Int, val end: Int, val score: Int)

/**
 * The verification code in [text] (a notification's title and text), with
 * how sure it is and who it's from, or null.
 *
 * The text must say it's a code (OTP, verification, passcode, one-time,
 * code, PIN... in English, Hindi "ओटीपी", "सत्यापन", "कोड" and other
 * languages). A code is a standalone 4–8 digit number ("123 456" and
 * "123-456" join into one; "G-123456" gives 123456), or 6–8 capitals and
 * digits next to a word that says "one-time code" (never in coupon talk).
 * Skipped: amounts (₹, Rs, INR, $, decimals), phone and account numbers
 * (9+ digits, after "call", masked "XX1234", "card ending 1234"), dates,
 * times, a year (only if nothing else fits), labelled order, reference,
 * ticket and booking numbers. The nearest to a keyword wins.
 */
fun detectCode(text: String, title: String = "", app: String = ""): CodeMatch? {
    val all = if (title.isBlank()) text else "$title\n$text"
    val strong = STRONG.findAll(all).map { it.range.first to it.range.last + 1 }.toList()
    val weak = WEAK.findAll(all).map { it.range.first to it.range.last + 1 }.toList()
    val keywords = strong + weak
    if (keywords.isEmpty()) return null
    val promo = PROMO.containsMatchIn(all)
    val talk = OTP_TALK.containsMatchIn(all)

    val runs = RUN.findAll(all).map { Run(it.range.first, it.range.last + 1, it.value) }.toList()
    val cands = mutableListOf<Cand>()

    fun context(start: Int, end: Int): Boolean =
        !NOT_CODE_BEFORE.containsMatchIn(all.substring(0, start)) &&
            !NOT_CODE_AFTER.containsMatchIn(all.substring(end))

    // Part of a bigger number: 1,500.00 / 02/11/2026 / 10:30 / +1 555...
    fun inBiggerNumber(start: Int, end: Int): Boolean {
        val b = all.getOrNull(start - 1)
        val bb = all.getOrNull(start - 2)
        val a = all.getOrNull(end)
        val aa = all.getOrNull(end + 1)
        val joiners = ".,/:-"
        return (b != null && b in joiners && bb?.isDigit() == true) ||
            (a != null && a in joiners && aa?.isDigit() == true) ||
            b == '+' || b == '*' || b == '₹' || b == '$'
    }

    fun distance(start: Int, end: Int, among: List<Pair<Int, Int>>): Int = among.minOfOrNull { (ks, ke) ->
        when {
            start >= ke -> start - ke
            ks >= end -> ks - end
            else -> 0
        }
    } ?: Int.MAX_VALUE

    fun score(code: String, start: Int, end: Int, base: Int, year: Boolean = false): Int {
        var s = base
        val d = distance(start, end, keywords)
        val rightAfter = keywords.any { (_, ke) -> ke <= start && RIGHT_AFTER_KEYWORD.matches(all.substring(ke, start)) }
        val rightBefore = keywords.any { (ks, _) -> ks >= end && ks - end <= 12 && all.substring(end, ks).trim().lowercase()
            .let { it.isEmpty() || it in setOf("is", "is your", "is the", "as your", "as", "है", "आपका", "your") || it.endsWith("is your") || it.endsWith("is the") } }
        s -= when {
            rightAfter || rightBefore -> 0
            d <= 20 -> 10
            d <= 60 -> 20
            d <= 120 -> 30
            else -> 45
        }
        if (strong.isEmpty()) s -= 15
        if (code.length != 6) s -= 5
        if (talk) s += 5
        // A year fits only if nothing else does, unless it sits right by the word.
        if (year) s -= if (rightAfter || rightBefore) 15 else 40
        return s.coerceIn(0, 100)
    }

    fun digitRun(r: Run) {
        if (r.len !in 4..8 || inBiggerNumber(r.start, r.end) || !context(r.start, r.end)) return
        val year = r.len == 4 && r.value.toInt() in 1900..2099
        cands += Cand(r.value, r.start, r.end, score(r.value, r.start, r.end, 100, year))
    }

    fun mixedRun(r: Run) {
        val digits = r.value.count { it in '0'..'9' }
        if (r.len !in 6..8 || digits == 0 || digits == r.len) return
        if (promo || strong.isEmpty()) return
        // Capitals and digits only (not a word like "iPhone15").
        if (r.value.any { it in 'a'..'z' }) return
        // Masked: XX1234, XXXX5678.
        if (r.value.filter { it.isLetter() }.all { it == 'X' }) return
        if (all.getOrNull(r.start - 1) == '*' || !context(r.start, r.end)) return
        // Only right next to a word that says it's a code.
        if (distance(r.start, r.end, strong + weak) > 25) return
        cands += Cand(r.value, r.start, r.end, score(r.value, r.start, r.end, 90))
    }

    var i = 0
    while (i < runs.size) {
        val r = runs[i]
        if (!r.digits) {
            // "G-123456": Google's prefix, not part of the code.
            val next = runs.getOrNull(i + 1)
            if (r.value == "G" && next != null && next.digits && next.start == r.end + 1 && all[r.end] == '-') {
                if (next.len in 4..8 && context(r.start, next.end)) {
                    cands += Cand(next.value, next.start, next.end, score(next.value, next.start, next.end, 100))
                }
                i += 2; continue
            }
            mixedRun(r); i++; continue
        }
        // Digit groups split by one space or dash: "123 456", "+91 98765 43210".
        var j = i
        while (j + 1 < runs.size && runs[j + 1].digits && runs[j].end + 1 == runs[j + 1].start &&
            all[runs[j].end] in " -"
        ) j++
        val group = runs.subList(i, j + 1)
        val total = group.sumOf { it.len }
        val (gs, ge) = group.first().start to group.last().end
        when {
            group.size == 1 -> digitRun(r)
            total >= 9 -> Unit // a phone or account number
            all.getOrNull(gs - 1) == '+' -> Unit
            group.all { it.len in 3..4 } && total in 6..8 && context(gs, ge) && !inBiggerNumber(gs, ge) -> {
                val code = group.joinToString("") { it.value }
                cands += Cand(code, gs, ge, score(code, gs, ge, 95))
            }
            else -> group.forEach(::digitRun)
        }
        i = j + 1
    }

    val best = cands
        .filter { isCode(it.code) || it.code.length in 6..8 }
        .maxWithOrNull(compareBy<Cand>({ it.score }, { -distance(it.start, it.end, keywords) }))
        ?: return null
    return CodeMatch(best.code, best.score, issuerOf(text, title, app))
}

/** The code alone (see [detectCode]); any confidence. */
fun findCode(text: String): String? = detectCode(text)?.code

/** Mirrors `mdcore::features::code::is_code`. */
fun isCode(code: String): Boolean =
    code.length in 4..10 &&
        code.all { it in 'a'..'z' || it in 'A'..'Z' || it in '0'..'9' } &&
        code.count { it in '0'..'9' } >= 4

// --- who it's from --------------------------------------------------------------

private const val NAME = "([A-Z][A-Za-z0-9&.'’]*(?:[ ][A-Z][A-Za-z0-9&.'’]*){0,2})"
private const val NAME_ANY = "([\\p{Lu}0-9][\\p{L}\\p{N}&.'’]*(?:[ ][\\p{Lu}0-9][\\p{L}\\p{N}&.'’]*){0,2})"

private val ISSUER_PATTERNS = listOf(
    // "[Amazon] 123456 is ..." / "<#> Amazon: ..."
    Regex("^\\s*(?:<#>\\s*)?\\[([^\\]\\n]{2,40})\\]"),
    // "Amazon: 246810 is your ..."
    Regex("^\\s*(?:<#>\\s*)?$NAME_ANY\\s*:\\s"),
    // "your Swiggy verification code", "your WhatsApp code", "your Amazon OTP"
    Regex("\\b(?i:your)\\s+$NAME\\s+(?i:(?:verification|login|log-in|sign-in|sign in|security|one-time|one time|confirmation|account)\\s+)?(?i:code|otp|pin|passcode)\\b"),
    // "for login to HDFC Bank NetBanking", "to complete your Paytm login"
    Regex("\\b(?i:to|for|on|at|into|with)\\s+(?:(?i:your)\\s+)?$NAME\\s+(?i:netbanking|net banking|account|login|log-in|sign-in|sign in|app|wallet|id|transaction|payment)\\b"),
    Regex("\\b(?i:your)\\s+$NAME\\s+(?i:netbanking|net banking|account|login|log-in|sign-in|sign in|app|wallet|id)\\b"),
    // "from Google", "Welcome to Zepto"
    Regex("\\b(?i:from|welcome to)\\s+$NAME"),
    // "... Do not share. -SBI" / "- Swiggy"
    Regex("[-–—]\\s*([A-Z][A-Za-z0-9&.' ]{1,30}?)\\s*\\.?\\s*$"),
    // Hindi: "एसबीआई का ओटीपी"
    Regex("([\\p{L}\\p{M}]{2,20}(?: [\\p{L}\\p{M}]{2,20})?) (?:का|की) (?:ओटीपी|कोड|सत्यापन कोड)"),
)

private val NOT_A_NAME = setOf(
    "your", "you", "the", "this", "that", "otp", "code", "pin", "dear", "customer", "user", "use", "login",
    "verification", "account", "one", "a", "an", "it", "never", "do", "dont", "don't", "we", "our", "my", "hi",
    "hello", "welcome", "valid", "please", "sms", "is", "for", "to", "from", "not", "share", "g", "pay", "payment",
    "transaction", "txn", "security", "alert", "passcode", "password", "new", "message", "आपका", "आपकी", "यह",
)

/** A messaging app's title is the sender: "VM-HDFCBK" -> "HDFCBK". */
private val SENDER_ID = Regex("^[A-Z]{2}-([A-Z0-9]{3,9})(?:-[A-Z])?$")
private val PHONE_LIKE = Regex("^[+\\d\\s()\\-]{5,}$")

/** Who a code is from: the name the text gives ("Your Swiggy code",
 *  "[Amazon]", "-SBI", "from Google"), else the sender in the title,
 *  else empty. Never the [app] itself (that's sent apart). */
fun issuerOf(text: String, title: String = "", app: String = ""): String {
    for (p in ISSUER_PATTERNS) {
        for (m in p.findAll(text)) {
            val name = tidyIssuer(m.groupValues[1])
            if (name.isNotEmpty() && !name.equals(app, ignoreCase = true)) return name
        }
    }
    val t = title.trim()
    if (t.isNotEmpty() && !PHONE_LIKE.matches(t) && !STRONG.containsMatchIn(t) && !WEAK.containsMatchIn(t)) {
        val sender = SENDER_ID.matchEntire(t)?.groupValues?.get(1) ?: t
        val name = tidyIssuer(sender)
        if (name.isNotEmpty() && !name.equals(app, ignoreCase = true) && name.length <= 30) return name
    }
    return ""
}

private fun tidyIssuer(raw: String): String {
    val s = raw.trim().trimEnd('.', ',', ':', ';', '-', '!', ' ').trim()
    if (s.length < 2 || s.length > 40) return ""
    if (s.all { it.isDigit() || it == ' ' }) return ""
    val words = s.lowercase().split(' ')
    if (words.first() in NOT_A_NAME || s.lowercase() in NOT_A_NAME) return ""
    if (STRONG.matches(s) || WEAK.matches(s)) return ""
    return s
}

/** Which computers a code goes to: the [chosen] ones (null: all of
 *  them) among [computers] (node id, takes codes), never one that doesn't
 *  take them. */
fun codeTargets(computers: List<Pair<String, Boolean>>, chosen: Set<String>?): List<String> =
    computers.filter { (node, ok) -> ok && (chosen == null || node in chosen) }.map { it.first }
