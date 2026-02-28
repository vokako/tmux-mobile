package com.tmuxmobile.dev

import android.os.Bundle
import android.view.ViewTreeObserver
import android.graphics.Rect
import androidx.activity.enableEdgeToEdge
import androidx.core.view.ViewCompat
import androidx.core.view.WindowInsetsCompat

class MainActivity : TauriActivity() {
  override fun onCreate(savedInstanceState: Bundle?) {
    enableEdgeToEdge()
    super.onCreate(savedInstanceState)

    val rootView = window.decorView.rootView

    // Send status bar + navigation bar insets to WebView
    ViewCompat.setOnApplyWindowInsetsListener(rootView) { view, insets ->
      val systemBars = insets.getInsets(WindowInsetsCompat.Type.systemBars())
      val webView = findWebView(rootView)
      webView?.evaluateJavascript("""
        document.documentElement.style.setProperty('--sat', '${systemBars.top}px');
        document.documentElement.style.setProperty('--sab', '${systemBars.bottom}px');
      """.trimIndent(), null)
      insets
    }

    // Keyboard height detection
    rootView.viewTreeObserver.addOnGlobalLayoutListener {
      val rect = Rect()
      rootView.getWindowVisibleDisplayFrame(rect)
      val screenHeight = rootView.height
      val keyboardHeight = screenHeight - rect.bottom
      val webView = findWebView(rootView)
      if (keyboardHeight > 150) {
        webView?.evaluateJavascript("""
          window.__ANDROID_KEYBOARD_HEIGHT__ = $keyboardHeight;
          window.dispatchEvent(new CustomEvent('androidKeyboardHeight', { detail: { height: $keyboardHeight } }));
        """.trimIndent(), null)
      } else {
        webView?.evaluateJavascript("""
          window.__ANDROID_KEYBOARD_HEIGHT__ = 0;
          window.dispatchEvent(new CustomEvent('androidKeyboardHeight', { detail: { height: 0 } }));
        """.trimIndent(), null)
      }
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
}
