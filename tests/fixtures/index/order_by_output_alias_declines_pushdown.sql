-- The `ORDER BY` index pushdown rewrites the scan to walk a **base column**'s index,
-- so it may only fire on a key that binds that base column — and its own gate tests
-- the key's *shape* (a bare `Expr::Column`) rather than its binding. `b AS a` makes
-- the whole bare key `a` name the output column, which Postgres's SQL92 rule binds to
-- the projected `b`; the pushdown bound it to indexed base `a` and walked the index,
-- answering the rows in exactly the wrong order with no error.
--
-- Two things make this worth a fixture of its own rather than a `select/` one. The
-- pushdown runs **before** `build_sort`, so it decided the output-name question ahead
-- of the code that answers it correctly — a fourth site, and the one that wins. And it
-- is a **build-profile split**: the stripped planner never pushes down, so it kept the
-- `Sort` and answered correctly, while the accelerated build did not. The identical
-- statement over an unindexed table was correct on both profiles, which is what
-- identifies the pushdown rather than the sort as the cause.
-- @profile-lock
-- @setup
CREATE TABLE oap (a INT NOT NULL, b INT NOT NULL);
CREATE INDEX oap_a ON oap(a);
INSERT INTO oap VALUES (1, 20), (2, 10);

-- @test
SELECT b AS a FROM oap ORDER BY a;
