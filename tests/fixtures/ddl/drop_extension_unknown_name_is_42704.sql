-- Without `IF EXISTS`, a `DROP EXTENSION` naming something this engine does not
-- provide is `42704`. The pair with
-- `drop_extension_if_exists_skips_an_unknown_name.sql` is what shows the `IF EXISTS`
-- flag is read rather than assumed.
-- @test
-- @expect-error: extension "nosuch" does not exist
DROP EXTENSION nosuch;
