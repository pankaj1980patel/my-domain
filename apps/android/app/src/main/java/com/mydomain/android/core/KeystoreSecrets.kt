package com.mydomain.android.core

import android.content.Context
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import android.util.Base64
import android.util.Log
import com.mydomain.android.ffi.SecretStore
import java.security.KeyStore
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
 * sealed bytes are kept in private SharedPreferences.
 */
class KeystoreSecrets(context: Context) : SecretStore {
    private val prefs = context.getSharedPreferences("md_secrets", Context.MODE_PRIVATE)

    private fun key(): SecretKey {
        val ks = KeyStore.getInstance(KEYSTORE).apply { load(null) }
        (ks.getKey(ALIAS, null) as? SecretKey)?.let { return it }
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
        return runCatching {
            val bytes = Base64.decode(sealed, Base64.NO_WRAP)
            val c = Cipher.getInstance(TRANSFORM)
            c.init(Cipher.DECRYPT_MODE, key(), GCMParameterSpec(128, bytes, 0, IV_LEN))
            String(c.doFinal(bytes, IV_LEN, bytes.size - IV_LEN), Charsets.UTF_8)
        }.onFailure { Log.w(TAG, "can't read secret $key: $it") }.getOrNull()
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
        }.onFailure { Log.w(TAG, "can't save secret $key: $it") }
    }

    private companion object {
        const val KEYSTORE = "AndroidKeyStore"
        const val ALIAS = "md_session"
        const val TRANSFORM = "AES/GCM/NoPadding"
        const val IV_LEN = 12
        const val TAG = "mydomain_secrets"
    }
}
