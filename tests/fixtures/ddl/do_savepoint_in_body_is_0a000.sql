-- Transaction control inside a `DO` body is refused, not silently ignored.
--
-- The lifecycle a transaction-control statement asks for is applied by the host,
-- which reads it off that statement's own result — and a `DO` block's inner
-- results never reach the host. So an accepted `SAVEPOINT` here would parse, run
-- to a command tag, change nothing, and leave the surrounding writes standing as
-- though it had worked. 18.6 answers `0A000 unsupported transaction command in
-- PL/pgSQL`, and so does this.
--
-- The fixture exists because the hole was *opened* by teaching the parser
-- savepoints: before that, `SAVEPOINT` was `0A000` from the statement dispatcher
-- and this case was right by accident.
-- @setup
CREATE TABLE do_tc (id INT);
-- @test
-- @expect-error: unsupported transaction command in PL/pgSQL (0A000)
DO $$ BEGIN
    SAVEPOINT s;
    INSERT INTO do_tc VALUES (1);
END $$;
