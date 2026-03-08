/**
 * Virtual scroll engine for efficiently rendering large lists.
 * Only renders visible items + a buffer zone, recycling DOM nodes.
 *
 * Usage:
 *   const vs = new VirtualScroll({
 *     container: document.getElementById('list'),
 *     itemHeight: 44,
 *     totalItems: 14000,
 *     renderItem: (index) => createEmailRow(index),
 *     bufferSize: 10,
 *   });
 *   vs.refresh(); // call after data changes
 */
class VirtualScroll {
  constructor({ container, itemHeight, totalItems, renderItem, bufferSize = 10 }) {
    this.container = container;
    this.itemHeight = itemHeight;
    this.totalItems = totalItems;
    this.renderItem = renderItem;
    this.bufferSize = bufferSize;

    // Internal state
    this._items = new Map(); // index -> DOM element
    this._scrollTop = 0;
    this._containerHeight = 0;

    // Setup DOM structure
    this.container.style.overflow = 'auto';
    this.container.style.position = 'relative';

    // Sentinel element to maintain correct scroll height
    this._sentinel = document.createElement('div');
    this._sentinel.style.height = `${totalItems * itemHeight}px`;
    this._sentinel.style.position = 'relative';
    this.container.innerHTML = '';
    this.container.appendChild(this._sentinel);

    // Viewport for rendered items
    this._viewport = document.createElement('div');
    this._viewport.style.position = 'absolute';
    this._viewport.style.top = '0';
    this._viewport.style.left = '0';
    this._viewport.style.right = '0';
    this._sentinel.appendChild(this._viewport);

    // Bind scroll handler
    this._onScroll = this._handleScroll.bind(this);
    this.container.addEventListener('scroll', this._onScroll, { passive: true });

    // Initial render
    this._render();
  }

  /** Update total items count and re-render */
  setTotalItems(count) {
    this.totalItems = count;
    this._sentinel.style.height = `${count * this.itemHeight}px`;
    this._render();
  }

  /** Force a full re-render (e.g., after data change) */
  refresh() {
    this._items.clear();
    this._viewport.innerHTML = '';
    this._render();
  }

  /** Scroll to a specific item index */
  scrollToIndex(index) {
    this.container.scrollTop = index * this.itemHeight;
  }

  /** Get the currently visible range */
  getVisibleRange() {
    const scrollTop = this.container.scrollTop;
    const height = this.container.clientHeight;
    const start = Math.floor(scrollTop / this.itemHeight);
    const end = Math.min(
      Math.ceil((scrollTop + height) / this.itemHeight),
      this.totalItems - 1
    );
    return { start, end };
  }

  /** Clean up event listeners */
  destroy() {
    this.container.removeEventListener('scroll', this._onScroll);
  }

  _handleScroll() {
    requestAnimationFrame(() => this._render());
  }

  _render() {
    const scrollTop = this.container.scrollTop;
    const height = this.container.clientHeight;

    if (height === 0) return; // Container not visible yet

    // Calculate visible range with buffer
    const startIdx = Math.max(0, Math.floor(scrollTop / this.itemHeight) - this.bufferSize);
    const endIdx = Math.min(
      this.totalItems - 1,
      Math.ceil((scrollTop + height) / this.itemHeight) + this.bufferSize
    );

    // Remove items outside the range
    for (const [idx, el] of this._items) {
      if (idx < startIdx || idx > endIdx) {
        el.remove();
        this._items.delete(idx);
      }
    }

    // Add items in range that aren't rendered yet
    for (let i = startIdx; i <= endIdx; i++) {
      if (!this._items.has(i)) {
        const el = this.renderItem(i);
        if (el) {
          el.style.position = 'absolute';
          el.style.top = `${i * this.itemHeight}px`;
          el.style.left = '0';
          el.style.right = '0';
          el.style.height = `${this.itemHeight}px`;
          this._viewport.appendChild(el);
          this._items.set(i, el);
        }
      }
    }
  }
}
