-- A sequence advanced inside a caught `DO` body is **not** rolled back with it:
-- `nextval` is non-transactional, so after a body that drew two values and then
-- raised, the next value is the third. Measured on 18.6.
--
-- The block's rollback shares one catalog-restore rule with the whole-block and
-- savepoint rollbacks (`Schema::restore_from`), and this fixture is the `DO`
-- member of the set that pins it — the rows the body wrote are discarded in the
-- same breath, which is what makes the counter's survival a rule rather than a
-- failure to roll anything back.
-- @setup
CREATE TABLE do_seq_t (id INT PRIMARY KEY);
CREATE SEQUENCE do_seq;
DO $$ BEGIN
    INSERT INTO do_seq_t VALUES (nextval('do_seq'));
    INSERT INTO do_seq_t VALUES (nextval('do_seq'));
    INSERT INTO do_seq_t VALUES (1);
EXCEPTION WHEN unique_violation THEN NULL;
END $$;

-- @test
SELECT nextval('do_seq') AS next_after_caught_do, (SELECT count(*) FROM do_seq_t) AS rows_kept;
