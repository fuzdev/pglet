-- The same refusal when the edge is *moved* rather than cleared. This route matters
-- on its own: re-pointing the sequence at another table makes it that table's AUTO
-- dependent, so `DROP TABLE <the other table>` takes it — the identity's sequence
-- removed by a statement that never names it.
-- @setup
CREATE TABLE aobot_host (h INT);
CREATE TABLE aobot_t (id INT GENERATED ALWAYS AS IDENTITY, b INT);
-- @test
ALTER SEQUENCE aobot_t_id_seq OWNED BY aobot_host.h;
-- @expect-error: cannot change ownership of identity sequence (0A000)
