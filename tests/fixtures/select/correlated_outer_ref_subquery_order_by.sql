-- The correlated outer reference in the **subquery's own `ORDER BY`** — a `Plan::Sort`
-- key, which the descent re-resolves like any other expression of that level.
-- @setup
CREATE TABLE co_o (okey INT, ouid UUID);
CREATE TABLE co_i (ikey INT, itag TEXT);
INSERT INTO co_o VALUES (1, '11111111-1111-1111-1111-111111111111'), (2, '22222222-2222-2222-2222-222222222222');
INSERT INTO co_i VALUES (1, 'a'), (2, 'b');
-- @test
SELECT okey FROM co_o WHERE EXISTS (SELECT 1 FROM co_i WHERE co_i.ikey = co_o.okey ORDER BY co_o.ouid = '11111111-1111-1111-1111-111111111111') ORDER BY okey;
