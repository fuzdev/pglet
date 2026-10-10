-- The `ADD COLUMN` route's "beats" half: the inline constraint block sits behind the
-- cook there too, so an ill-typed inline `CHECK` loses to the default's `22P02`.
-- @setup
CREATE TABLE add_ck (a INT);
-- @expect-error: the default is cooked before the inline CHECK is type-checked
-- @test
ALTER TABLE add_ck ADD COLUMN b INT DEFAULT 'x' CHECK (b > 'zz');
