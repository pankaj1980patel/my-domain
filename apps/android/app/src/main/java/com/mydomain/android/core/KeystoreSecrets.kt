package com.mydomain.android.core

import android.content.Context
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyPermanentlyInvalidatedException
import android.security.keystore.KeyProperties
import android.util.Base64
import android.util.Log
import com.mydomain.android.ffi.SecretStore
import java.security.KeyStore
import javax.crypto.AEADBadTagException
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec

/**
 * The core's secret store (session token, derived device key), so the login
 * survives the app being killed and a push can wake a signed-in engine.
 *
 * Each value is sealed with AES-GCM under a key that lives in the Android
 * Keystore (it never leaves the secure hardware where there is one), and the
 * sealed bytes are kept in private SharedPreferences. Neither is backed up
 * (`allowBackup` is off in the manifest): the Keystore key never leaves the
 * phone, so sealed bytes restored elsewhere could never be opened.
 *
 * The rule for a sealed value that can't be opened: it is deleted and
 * reported [lost] (so the core makes a fresh one) only when it can *never*
 * be opened again, which is one of:
 *  - the Keystore has no key under [ALIAS] although a value was sealed with
 *    it (the app's data came from a restore or a transfer, or the Keystore
 *    was wiped);
 *  - the Keystore says the key is permanently invalid
 *    ([KeyPermanentlyInvalidatedException]);
 *  - the bytes don't open with the key ([AEADBadTagException]: sealed under
 *    another key), or aren't sealed bytes at all.
 * Any other failure (the Keystore busy or failing for a moment) is
 * transient: the value is kept, `get` says "none for now", and the next
 * start tries again. Deleting on a transient error would throw away this
 * phone's identity for good.
 */
class KeystoreSecrets(context: Context) : SecretStore {
    private val prefs = context.getSharedPreferences("md_secrets", Context.MODE_PRIVATE)
    /** Keys whose sealed value was found unreadable for good (and deleted). */
    private val lostPrefs = context.getSharedPreferences("md_secrets_lost", Context.MODE_PRIVATE)

    private fun keystore(): KeyStore = KeyStore.getInstance(KEYSTORE).apply { load(null) }

    /** The sealing key, made when there is none (only when sealing). */
    private fun key(): SecretKey {
        (keystore().getKey(ALIAS, null) as? SecretKey)?.let { return it }
        val gen = KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, KEYSTORE)
        gen.init(
            KeyGenParameterSpec.Builder(ALIAS, KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT)
                .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
                .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
                .setKeySize(256)
                .build(),
        )
        return gen.generateKey()
    }

    override fun get(key: String): String? {
        val sealed = prefs.getString(key, null) ?: return null
        return try {
            // Opening never makes a key: a missing one means the value can't
            // be opened, ever (a new key wouldn't be the one that sealed it).
            val k = keystore().getKey(ALIAS, null) as? SecretKey ?: throw MissingKey()
            val bytes = Base64.decode(sealed, Base64.NO_WRAP)
            if (bytes.size <= IV_LEN) throw IllegalArgumentException("too short to be sealed")
            val c = Cipher.getInstance(TRANSFORM)
            c.init(Cipher.DECRYPT_MODE, k, GCMParameterSpec(128, bytes, 0, IV_LEN))
            String(c.doFinal(bytes, IV_LEN, bytes.size - IV_LEN), Charsets.UTF_8)
        } catch (e: Exception) {
            if (lostForGood(e)) {
                Log.w(TAG, "secret $key can never be read again (${e.javaClass.simpleName}): deleting it")
                prefs.edit().remove(key).apply()
                lostPrefs.edit().putBoolean(key, true).apply()
            } else {
                Log.w(TAG, "can't read secret $key for now: $e")
            }
            null
        }
    }

    override fun set(key: String, value: String?) {
        if (value == null) {
            prefs.edit().remove(key).apply()
            return
        }
        runCatching {
            val c = Cipher.getInstance(TRANSFORM)
            c.init(Cipher.ENCRYPT_MODE, key())
            val sealed = c.iv + c.doFinal(value.toByteArray(Charsets.UTF_8))
            prefs.edit().putString(key, Base64.encodeToString(sealed, Base64.NO_WRAP)).apply()
            // A new value is in: it's no longer lost.
            lostPrefs.edit().remove(key).apply()
        }.onFailure { Log.w(TAG, "can't save secret $key: $it") }
    }

    override fun lost(key: String): Boolean {
        // Reading is what finds out (and records) a value lost for good.
        if (prefs.contains(key)) get(key)
        return lostPrefs.getBoolean(key, false)
    }

    private class MissingKey : Exception("no Keystore key")

    /** The rule above: only these mean the value can never be opened. */
    private fun lostForGood(e: Exception): Boolean = when (e) {
        is MissingKey, is KeyPermanentlyInvalidatedException, is AEADBadTagException -> true
        // Not base64, or too short to be sealed bytes: garbage, for good.
        is IllegalArgumentException -> true
        else -> false
    }

    private companion object {
        const val KEYSTORE = "AndroidKeyStore"
        const val ALIAS = "md_session"
        const val TRANSFORM = "AES/GCM/NoPadding"
        const val IV_LEN = 12
        const val TAG = "mydomain_secrets"
    }
}
