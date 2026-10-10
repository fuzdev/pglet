-- With `TO`, `SET NAMES` is `generic_set` naming a setting called `names`, and 18.6 reads the list
-- before it looks the setting up: two values are its `22023 SET names takes only one argument`,
-- ahead of the `42704` one value gets (`set_names_to_loud.sql`).
-- @test
-- @expect-error: SET names takes only one argument
SET NAMES TO 'a', 'b';
