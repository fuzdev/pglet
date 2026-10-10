-- The `CHECK`'s reject side, which the value fixture beside it cannot express: a row
-- whose value equals the constant's **whole** literal violates the constraint. Under
-- the truncation the stored predicate read `c <> 'z'`, so this row was accepted.
-- @setup
CREATE TABLE cck (c TEXT CHECK (c <> char 'zz'));
-- @test
-- @expect-error: new row violates check constraint
INSERT INTO cck VALUES ('zz');
