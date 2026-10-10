-- And on the `SET DEFAULT` route the column is resolved before the default is cooked
-- (`42703`), which together with the `ALTER` route's `42P01` fixes both of the two
-- gates that outrank the cook there.
-- @setup
CREATE TABLE sd_prec (a INT);
-- @expect-error: the column is resolved before the default is cooked
-- @test
ALTER TABLE sd_prec ALTER COLUMN nosuchcol SET DEFAULT 'x';
