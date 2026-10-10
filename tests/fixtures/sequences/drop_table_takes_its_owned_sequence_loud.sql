-- A `SERIAL`'s sequence is `OWNED BY` the column it feeds, so `DROP TABLE` takes it
-- (the AUTO edge) and the name resolves to nothing afterwards. Leaving it behind was
-- silent: every read still worked, and the cost was one burned sequence name per
-- drop/create cycle.
-- @setup
CREATE TABLE dttos_t (a SERIAL, b INT);
DROP TABLE dttos_t;
-- @test
SELECT nextval('dttos_t_a_seq');
-- @expect-error: the owned sequence went with its table (42P01)
