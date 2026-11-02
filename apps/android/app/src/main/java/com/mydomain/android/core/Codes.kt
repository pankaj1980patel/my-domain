package com.mydomain.android.core

/*
 * Finding a one-time verification code in a notification's text, so it can
 * be sent to the computers (`MdEngine.shareCode`). Pure Kotlin: unit-tested
 * on the JVM (src/test/.../CodesTest.kt).
 */

/** Words that say "this text carries a code" (English, plus common Hindi). */
private val KEYWORD = Regex(
    """(?i)\b(?:otp|codes?|passcode|pass code|verification|verify|one[- ]?time|pin|2fa)\b|कोड|ओटीपी""",
)

/** Just before a number: it's money, or an account/card/reference number. */
private val NOT_CODE_BEFORE = Regex(
    """(?i)(?:₹|\$|\brs\.?|\binr|\busd|\ba/c|\bacct|\baccount|\bcard|\bending(?: with| in)?|\bref(?:erence)?|\btxn|\border|\bno\.?|\bid|#)\s*[:.]?\s*$""",
)

/** Just after a number: it's money. */
private val NOT_CODE_AFTER = Regex("""(?i)^(?:\s*(?:rs\b|inr\b|rupees|usd\b)|/-)""")

private val RUN = Regex("[A-Za-z0-9]+")

private class Run(val start: Int, val end: Int, val value: String) {
    val len get() = value.length
    val digits = value.all(Char::isDigit)
}

private class Cand(val code: String, val start: Int, val end: Int, val yearLike: Boolean)

/**
 * The verification code in [text] (a notification's title + text), or null.
 *
 * The text must mention a code (code, OTP, verification, passcode,
 * one-time, PIN; Hindi "कोड", "ओटीपी"). The code is a standalone 4–8 digit
 * number ("123 456" / "123-456" join into one), or a short letters+digits
 * token; the part after "G-" in "G-123456". Amounts (₹, Rs, INR, $), phone
 * and account numbers (9+ digits, masked "XX1234", "card ending 1234"),
 * dates and times are skipped. With several candidates, the one nearest a
 * keyword wins (a year like 2026 only if nothing else fits).
 *
 * Anything returned passes the core's `is_code` (4–10 ASCII letters or
 * digits, at least 4 digits).
 */
fun findCode(text: String): String? {
    val keywords = KEYWORD.findAll(text).map { it.range.first to it.range.last + 1 }.toList()
    if (keywords.isEmpty()) return null

    val runs = RUN.findAll(text).map { Run(it.range.first, it.range.last + 1, it.value) }.toList()
    val cands = mutableListOf<Cand>()

    fun context(start: Int, end: Int): Boolean =
        !NOT_CODE_BEFORE.containsMatchIn(text.substring(0, start)) &&
            !NOT_CODE_AFTER.containsMatchIn(text.substring(end))

    // Part of a bigger number: 1,500.00 / 02/11/2026 / 10:30 / 2026-11-02 / +1 555...
    fun inBiggerNumber(r: Run): Boolean {
        val b = text.getOrNull(r.start - 1)
        val bb = text.getOrNull(r.start - 2)
        val a = text.getOrNull(r.end)
        val aa = text.getOrNull(r.end + 1)
        val joiners = ".,/:-"
        return (b != null && b in joiners && bb?.isDigit() == true) ||
            (a != null && a in joiners && aa?.isDigit() == true) ||
            b == '+' || b == '*'
    }

    fun digitRun(r: Run) {
        if (r.len !in 4..8 || inBiggerNumber(r) || !context(r.start, r.end)) return
        val year = r.len == 4 && r.value.toInt() in 1900..2099
        cands += Cand(r.value, r.start, r.end, year)
    }

    fun mixedRun(r: Run) {
        val digits = r.value.count(Char::isDigit)
        val letters = r.len - digits
        if (r.len !in 4..10 || digits < 4 || letters > digits) return
        // Masked: XX1234, xxxx5678.
        if (r.value.filter(Char::isLetter).all { it == 'x' || it == 'X' }) return
        if (text.getOrNull(r.start - 1) == '*' || !context(r.start, r.end)) return
        cands += Cand(r.value, r.start, r.end, false)
    }

    var i = 0
    while (i < runs.size) {
        val r = runs[i]
        if (!r.digits) {
            mixedRun(r); i++; continue
        }
        // Digit groups split by one space or dash: "123 456", "+91 98765 43210".
        var j = i
        while (j + 1 < runs.size && runs[j + 1].digits && runs[j].end + 1 == runs[j + 1].start &&
            text[runs[j].end] in " -"
        ) j++
        val group = runs.subList(i, j + 1)
        when {
            group.size == 1 -> digitRun(r)
            group.sumOf { it.len } >= 9 -> Unit // a phone or account number
            group.all { it.len == 3 } && context(group.first().start, group.last().end) ->
                cands += Cand(group.joinToString("") { it.value }, group.first().start, group.last().end, false)
            else -> group.forEach(::digitRun)
        }
        i = j + 1
    }

    fun distance(c: Cand): Int = keywords.minOf { (ks, ke) ->
        when {
            c.start >= ke -> c.start - ke
            ks >= c.end -> ks - c.end
            else -> 0
        }
    }

    return cands
        .filter { isCode(it.code) }
        .minByOrNull { distance(it) + if (it.yearLike) 100_000 else 0 }
        ?.code
}

/** Mirrors `mdcore::features::code::is_code`. */
fun isCode(code: String): Boolean =
    code.length in 4..10 &&
        code.all { it in 'a'..'z' || it in 'A'..'Z' || it in '0'..'9' } &&
        code.count { it in '0'..'9' } >= 4
