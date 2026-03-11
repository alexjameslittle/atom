package build.atom.hello

import android.app.Activity
import android.text.Editable
import android.text.TextWatcher
import android.util.TypedValue
import android.view.Gravity
import android.view.View
import android.widget.Button
import android.widget.EditText
import android.widget.LinearLayout
import android.widget.TextView

class AtomHostViewFactoryImpl : AtomHostViewFactory {
  override fun build(activity: Activity): View {
    val state = DemoState()
    val layout =
        LinearLayout(activity).apply {
          orientation = LinearLayout.VERTICAL
          gravity = Gravity.TOP or Gravity.CENTER_HORIZONTAL
          setPadding(48, 96, 48, 48)
        }
    val statusView =
        TextView(activity).apply {
          setTextSize(TypedValue.COMPLEX_UNIT_SP, 14f)
          gravity = Gravity.CENTER
          alpha = 0.8f
          contentDescription = DemoElement.STATUS.id
        }
    val inputView =
        EditText(activity).apply {
          hint = "Type something"
          contentDescription = DemoElement.INPUT.id
        }
    val echoView =
        TextView(activity).apply {
          gravity = Gravity.CENTER
          alpha = 0.7f
          contentDescription = DemoElement.ECHO.id
        }

    layout.addView(
        TextView(activity).apply {
          text = "Hello Atom"
          textSize = 24f
          gravity = Gravity.CENTER
          contentDescription = DemoElement.TITLE.id
        })
    layout.addView(
        TextView(activity).apply {
          text = "hello-atom"
          textSize = 14f
          gravity = Gravity.CENTER
          alpha = 0.6f
          contentDescription = DemoElement.SLUG.id
        })
    layout.addView(statusView)
    layout.addView(inputView)
    layout.addView(
        Button(activity).apply {
          text = "Primary Action"
          contentDescription = DemoElement.BUTTON.id
          setOnClickListener {
            state.tapCount += 1
            state.statusText = "primary-button-tapped-${state.tapCount}"
            render(state, statusView, echoView)
          }
        })
    layout.addView(echoView)

    inputView.addTextChangedListener(
        object : TextWatcher {
          override fun beforeTextChanged(
              sequence: CharSequence?,
              start: Int,
              count: Int,
              after: Int
          ) = Unit

          override fun onTextChanged(sequence: CharSequence?, start: Int, before: Int, count: Int) {
            state.echoText = "Typed: ${sequence ?: ""}"
            state.statusText = "typed-text"
            render(state, statusView, echoView)
          }

          override fun afterTextChanged(editable: Editable?) = Unit
        })

    render(state, statusView, echoView)
    return layout
  }

  private fun render(state: DemoState, statusView: TextView, echoView: TextView) {
    statusView.text = state.statusText
    echoView.text = state.echoText
  }
}

private enum class DemoElement(val id: String) {
  TITLE("atom.demo.title"),
  SLUG("atom.demo.slug"),
  STATUS("atom.demo.status"),
  INPUT("atom.demo.input"),
  BUTTON("atom.demo.primary_button"),
  ECHO("atom.demo.echo"),
}

private data class DemoState(
    var statusText: String = "ready",
    var echoText: String = "Typed: ",
    var tapCount: Int = 0,
)
