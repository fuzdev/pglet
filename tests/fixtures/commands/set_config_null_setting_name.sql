-- `set_config(NULL, …)` is `22004 SET requires parameter name` on 18.6, raised
-- before the name is looked up. pglet used to echo the *value* argument and report
-- success — a `SELECT` that claimed to have set something.
-- @test
-- @expect-error: SET requires parameter name
SELECT set_config(NULL, 'x', false);
