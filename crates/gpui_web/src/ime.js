const positions = new WeakMap();

export function configureImeInput(input) {
    input.style.cssText = "all: initial; position: fixed; top: 0; left: 0; width: 1px; height: 1px; min-width: 0; min-height: 0; margin: 0; padding: 0; border: 0; outline: 0; box-sizing: border-box; opacity: 0; pointer-events: none; font-size: 1px; line-height: 1px; text-indent: 0; direction: ltr;";
    input.spellcheck = false;
}

export function positionImeInput(canvas, input, x, y, height, logicalWidth, logicalHeight) {
    if (!(logicalWidth > 0 && logicalHeight > 0)) return;
    const rect = canvas.getBoundingClientRect();
    if (!(rect.width > 0 && rect.height > 0)) return;
    const style = getComputedStyle(canvas);
    const pixels = name => parseFloat(style[name]) || 0;
    const left = pixels("borderLeftWidth") + pixels("paddingLeft");
    const right = pixels("borderRightWidth") + pixels("paddingRight");
    const top = pixels("borderTopWidth") + pixels("paddingTop");
    const bottom = pixels("borderBottomWidth") + pixels("paddingBottom");
    const borderBox = style.boxSizing === "border-box";
    const width = pixels("width") + (borderBox ? 0 : left + right);
    const boxHeight = pixels("height") + (borderBox ? 0 : top + bottom);
    if (!(width > left + right && boxHeight > top + bottom)) return;
    const scaleX = rect.width / width;
    const scaleY = rect.height / boxHeight;
    // GPUI bounds and DOM geometry are logical pixels; DPR is already accounted
    // for by the renderer. Only the canvas content-to-viewport mapping belongs here.
    const contentScaleX = (width - left - right) * scaleX / logicalWidth;
    const contentScaleY = (boxHeight - top - bottom) * scaleY / logicalHeight;
    const position = [
        rect.left + left * scaleX + x * contentScaleX,
        rect.top + top * scaleY + y * contentScaleY,
        Math.max(1, height * contentScaleY),
    ];
    if (!position.every(Number.isFinite)) return;
    const previous = positions.get(input);
    if (previous && position.every((value, i) => value === previous[i])) return;
    positions.set(input, position);
    input.style.left = `${position[0]}px`;
    input.style.top = `${position[1]}px`;
    input.style.height = `${position[2]}px`;
    input.style.fontSize = `${position[2]}px`;
    input.style.lineHeight = `${position[2]}px`;
}
