-- The FK half of the same block, which 18.6 resolves after the CHECKs and the
-- constraint indexes — all of them behind the cook.
-- @expect-error: the default is cooked before the foreign key's parent is resolved
-- @test
CREATE TABLE def_fk (a INT DEFAULT 'x', b INT REFERENCES def_no_such_parent(id));
