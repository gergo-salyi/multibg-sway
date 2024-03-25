# Changelog

## 0.1.6 - 2024-03-25
- Fix displaying the wallpapers on outputs with fractional scale factor. This may help with [#5](https://github.com/gergo-salyi/multibg-sway/issues/5)

## 0.1.5 - 2024-01-02
- Fix displaying the wallpapers on outputs with higher than 1 integer scale factor. This may help with [#4](https://github.com/gergo-salyi/multibg-sway/issues/4)

## 0.1.4 - 2023-08-31
- Allocate/release graphics memory per output when the output is connected/disconnected. This may help with [#2](https://github.com/gergo-salyi/multibg-sway/issues/2)
- Log graphics memory use (our wayland shared memory pool sizes)
- Minor fix to avoid a logged error on redrawing backgrounds already being drawn
- Update dependencies

## 0.1.3 - 2023-05-05
- Update dependencies, including fast_image_resize which fixed a major bug

## 0.1.2 - 2023-04-27
- Fix crash on suspend [#1](https://github.com/gergo-salyi/multibg-sway/issues/1)
- Implement automatic image resizing

## 0.1.1
- Initial release
