package com.tmuxmobile.dev

import android.os.Bundle
import android.graphics.Rect
import android.content.Intent
import android.webkit.JavascriptInterface
import androidx.activity.enableEdgeToEdge
import androidx.core.view.ViewCompat
import androidx.core.view.WindowInsetsCompat
import androidx.core.content.FileProvider
import java.io.File

class MainActivity : TauriActivity() {
  override fun onCreate(savedInstanceState: Bundle?) {
    enableEdgeToEdge()
    super.onCreate(savedInstanceState)

    val rootView = window.decorView.rootView
    val density = resources.displayMetrics.density

    // Add JS interface for opening files (retry until WebView is in hierarchy)
    var attachAttempts = 0
    fun attachFileOpener() {
      val webView = findWebView(rootView)
      if (webView != null) {
        webView.addJavascriptInterface(FileOpener(), "AndroidFileOpener")
      } else if (++attachAttempts < 50) {
        rootView.postDelayed(::attachFileOpener, 100)
      }
    }
    rootView.post(::attachFileOpener)

    // Send status bar + navigation bar insets to WebView (convert to CSS px)
    ViewCompat.setOnApplyWindowInsetsListener(rootView) { _, insets ->
      val systemBars = insets.getInsets(WindowInsetsCompat.Type.systemBars())
      val satCss = (systemBars.top / density).toInt()
      val sabCss = (systemBars.bottom / density).toInt()
      val webView = findWebView(rootView)
      webView?.evaluateJavascript("""
        document.documentElement.style.setProperty('--sat', '${satCss}px');
        document.documentElement.style.setProperty('--sab', '${sabCss}px');
      """.trimIndent(), null)
      insets
    }

    // Keyboard height detection (convert to CSS px)
    var lastKeyboardCss = 0
    rootView.viewTreeObserver.addOnGlobalLayoutListener {
      val rect = Rect()
      rootView.getWindowVisibleDisplayFrame(rect)
      val screenHeight = rootView.height
      val keyboardPx = screenHeight - rect.bottom
      val keyboardCss = (keyboardPx / density).toInt()
      if (keyboardCss == lastKeyboardCss) return@addOnGlobalLayoutListener
      lastKeyboardCss = keyboardCss
      val webView = findWebView(rootView)
      val height = if (keyboardCss > 80) keyboardCss else 0
      webView?.evaluateJavascript("""
        window.__ANDROID_KEYBOARD_HEIGHT__ = $height;
        window.__dbg?.('android kb: keyboardPx=$keyboardPx keyboardCss=$keyboardCss height=$height screenH=${screenHeight} rectB=${rect.bottom} density=$density');
        window.dispatchEvent(new CustomEvent('androidKeyboardHeight', { detail: { height: $height } }));
      """.trimIndent(), null)
    }
  }

  private fun findWebView(view: android.view.View): android.webkit.WebView? {
    if (view is android.webkit.WebView) return view
    if (view is android.view.ViewGroup) {
      for (i in 0 until view.childCount) {
        val found = findWebView(view.getChildAt(i))
        if (found != null) return found
      }
    }
    return null
  }

  inner class FileOpener {
    @JavascriptInterface
    fun openFile(filePath: String): String {
      return try {
        val file = File(filePath)
        if (!file.exists()) return "File not found: $filePath"
        val uri = FileProvider.getUriForFile(
          this@MainActivity,
          "${applicationContext.packageName}.fileprovider",
          file
        )
        val mime = contentResolver.getType(uri) ?: "*/*"
        val intent = Intent(Intent.ACTION_VIEW).apply {
          setDataAndType(uri, mime)
          addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
          addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
        }
        startActivity(intent)
        "ok"
      } catch (e: Exception) {
        e.message ?: "unknown error"
      }
    }
  }
}
