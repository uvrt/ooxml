Lane RD2: `ooxml_core.CallContext` takes the host's limits as one `limits=ooxml_core.Limits(...)`
in place of the `max_result_chars=` and `max_list_items=` keywords. `ooxml_core.Limits` also
carries `max_images_per_call` (4) and `max_image_edge` (2,576 pixels), which `render` draws
within.
