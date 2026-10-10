-- A NULL value is a **reset**, and `set_config` returns the restored value rather
-- than NULL — measured on 18.6. pglet echoed the NULL. The restored value is
-- representable here because pglet's `search_path` is a fixed string, so this is a
-- value comparison and not an `@expect-error`.
-- @test
SELECT set_config('search_path', NULL, false) AS sp;
