/* @ds-bundle: {"format":4,"namespace":"VideoAutomation","components":[{"name":"Button"},{"name":"Input"},{"name":"Badge"},{"name":"Card"},{"name":"Alert"},{"name":"ProgressBar"}]} */
(function () {
  var h = window.React.createElement;

  function cx() {
    var out = [];
    for (var i = 0; i < arguments.length; i++) {
      if (arguments[i]) out.push(arguments[i]);
    }
    return out.join(' ');
  }

  function omit(obj, keys) {
    var rest = {};
    for (var k in obj) {
      if (Object.prototype.hasOwnProperty.call(obj, k) && keys.indexOf(k) === -1) rest[k] = obj[k];
    }
    return rest;
  }

  function Button(props) {
    props = props || {};
    var variant = props.variant || 'primary';
    var size = props.size || 'md';
    var rest = omit(props, ['variant', 'size', 'className', 'children']);
    rest.className = cx('va-btn', 'va-btn-' + variant, 'va-btn-' + size, props.className);
    return h('button', rest, props.children);
  }

  function Input(props) {
    props = props || {};
    var label = props.label;
    var helperText = props.helperText;
    var error = props.error;
    var id = props.id || ('va-input-' + Math.random().toString(36).slice(2, 8));
    var rest = omit(props, ['label', 'helperText', 'error', 'id', 'className']);
    rest.id = id;
    rest.className = cx('va-input', error ? 'va-input-error' : null, props.className);
    var children = [];
    if (label) children.push(h('label', { key: 'l', htmlFor: id, className: 'va-input-label' }, label));
    children.push(h('input', Object.assign({ key: 'i' }, rest)));
    if (error && typeof error === 'string') {
      children.push(h('p', { key: 'e', className: 'va-input-helper va-input-helper-error' }, error));
    } else if (helperText) {
      children.push(h('p', { key: 'h', className: 'va-input-helper' }, helperText));
    }
    return h('div', { className: 'va-field' }, children);
  }

  function Badge(props) {
    props = props || {};
    var tone = props.tone || 'neutral';
    return h('span', { className: cx('va-badge', 'va-badge-' + tone) }, props.children);
  }

  function Card(props) {
    props = props || {};
    var padded = props.padded !== false;
    var rest = omit(props, ['padded', 'className', 'children']);
    rest.className = cx('va-card', padded ? 'va-card-padded' : null, props.className);
    return h('div', rest, props.children);
  }

  function Alert(props) {
    props = props || {};
    var tone = props.tone || 'info';
    var title = props.title;
    var children = [];
    if (title) children.push(h('p', { key: 't', className: 'va-alert-title' }, title));
    if (props.children) children.push(h('div', { key: 'c', className: 'va-alert-body' }, props.children));
    var extra = tone === 'danger' ? { role: 'alert' } : {};
    return h('div', Object.assign({ className: cx('va-alert', 'va-alert-' + tone) }, extra), children);
  }

  function ProgressBar(props) {
    props = props || {};
    var value = typeof props.value === 'number' ? Math.max(0, Math.min(100, props.value)) : 0;
    var indeterminate = !!props.indeterminate;
    var label = props.label;
    var children = [];
    if (label) {
      children.push(
        h('div', { key: 'l', className: 'va-progress-label' },
          h('span', null, label),
          indeterminate ? null : h('span', { className: 'va-progress-value' }, value + '%')
        )
      );
    }
    children.push(
      h('div', { key: 't', className: 'va-progress-track' },
        h('div', {
          className: cx('va-progress-fill', indeterminate ? 'va-progress-indeterminate' : null),
          style: indeterminate ? undefined : { width: value + '%' }
        })
      )
    );
    return h('div', { className: 'va-progress' }, children);
  }

  var api = { Button: Button, Input: Input, Badge: Badge, Card: Card, Alert: Alert, ProgressBar: ProgressBar };
  window.VideoAutomation = Object.assign(window.VideoAutomation || {}, api);
})();
