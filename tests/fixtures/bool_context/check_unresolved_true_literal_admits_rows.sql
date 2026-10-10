-- Its accept-side control, through the same predicate: `CHECK ('t')` admits the
-- row. A resolver that made a non-boolean CHECK violate unconditionally would
-- redden here.
-- @setup
CREATE TABLE bcck2 (a INT CHECK ('t'));
-- @test
INSERT INTO bcck2 VALUES (9) RETURNING a;
