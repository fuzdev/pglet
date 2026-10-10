/**
 * Drizzle ORM tests for pglet compatibility.
 *
 * Self-contained: Automatically spawns pglet server if not running.
 *
 * Tests basic CRUD operations using Drizzle ORM against pglet.
 * Run: deno task test:drizzle
 *
 * Status: All tests passing! Extended query protocol (Parse/Bind/Execute)
 * and parameterized queries are fully implemented.
 */

import { assertEquals } from "@std/assert";
import { drizzle } from "drizzle-orm/node-postgres";
import { and, asc, desc, eq, gt, or } from "drizzle-orm";
import { sql } from "drizzle-orm";
import pg from "pg";
import { posts, users } from "./schema.ts";
import { PORTS } from "../../lib/ports.ts";
import { ensureServer } from "../../lib/server.ts";

const { Client } = pg;

const CONFIG = {
  host: "127.0.0.1",
  port: PORTS.PGLET_TEST,
  user: "postgres",
  database: "postgres",
};

// Ensure server is running before tests
const cleanupServer = await ensureServer();

// Register cleanup
globalThis.addEventListener("unload", () => {
  cleanupServer();
});

/**
 * Helper to create a fresh connection with Drizzle.
 * Data now persists across connections (shared state).
 */
async function withDb<T>(
  fn: (db: ReturnType<typeof drizzle>) => Promise<T>,
): Promise<T> {
  const client = new Client(CONFIG);
  await client.connect();
  const db = drizzle(client);
  try {
    return await fn(db);
  } finally {
    await client.end();
  }
}

/**
 * Setup tables using raw SQL.
 * Drops tables first since data persists across connections.
 */
async function setupTables(db: ReturnType<typeof drizzle>): Promise<void> {
  // Drop existing tables first (data persists across connections)
  await db.execute(sql`DROP TABLE IF EXISTS posts CASCADE`);
  await db.execute(sql`DROP TABLE IF EXISTS users CASCADE`);
  await db.execute(sql`
    CREATE TABLE users (
      id INT PRIMARY KEY,
      name TEXT NOT NULL,
      email TEXT NOT NULL,
      active BOOL DEFAULT true,
      created_at TIMESTAMP
    )
  `);
  await db.execute(sql`
    CREATE TABLE posts (
      id INT PRIMARY KEY,
      title TEXT NOT NULL,
      content TEXT,
      author_id INT NOT NULL,
      published BOOL DEFAULT false
    )
  `);
}

// =============================================================================
// INSERT Tests
// =============================================================================

Deno.test("drizzle - insert single row", async () => {
  await withDb(async (db) => {
    await setupTables(db);

    await db.insert(users).values({
      id: 1,
      name: "Alice",
      email: "alice@example.com",
    });

    const result = await db.select().from(users);
    assertEquals(result.length, 1);
    assertEquals(result[0].name, "Alice");
    assertEquals(result[0].email, "alice@example.com");
  });
});

Deno.test("drizzle - insert multiple rows", async () => {
  await withDb(async (db) => {
    await setupTables(db);

    await db.insert(users).values([
      { id: 1, name: "Alice", email: "alice@example.com" },
      { id: 2, name: "Bob", email: "bob@example.com" },
      { id: 3, name: "Carol", email: "carol@example.com" },
    ]);

    const result = await db.select().from(users);
    assertEquals(result.length, 3);
  });
});

// =============================================================================
// SELECT Tests
// =============================================================================

Deno.test("drizzle - select all", async () => {
  await withDb(async (db) => {
    await setupTables(db);
    await db.insert(users).values([
      { id: 1, name: "Alice", email: "alice@example.com", active: true },
      { id: 2, name: "Bob", email: "bob@example.com", active: false },
    ]);

    const result = await db.select().from(users);
    assertEquals(result.length, 2);
  });
});

Deno.test("drizzle - select with where eq", async () => {
  await withDb(async (db) => {
    await setupTables(db);
    await db.insert(users).values([
      { id: 1, name: "Alice", email: "alice@example.com" },
      { id: 2, name: "Bob", email: "bob@example.com" },
    ]);

    const result = await db.select().from(users).where(eq(users.id, 1));
    assertEquals(result.length, 1);
    assertEquals(result[0].name, "Alice");
  });
});

Deno.test("drizzle - select with where gt", async () => {
  await withDb(async (db) => {
    await setupTables(db);
    await db.insert(users).values([
      { id: 1, name: "Alice", email: "alice@example.com" },
      { id: 2, name: "Bob", email: "bob@example.com" },
      { id: 3, name: "Carol", email: "carol@example.com" },
    ]);

    const result = await db.select().from(users).where(gt(users.id, 1));
    assertEquals(result.length, 2);
  });
});

Deno.test("drizzle - select with and", async () => {
  await withDb(async (db) => {
    await setupTables(db);
    await db.insert(users).values([
      { id: 1, name: "Alice", email: "alice@example.com", active: true },
      { id: 2, name: "Bob", email: "bob@example.com", active: false },
      { id: 3, name: "Carol", email: "carol@example.com", active: true },
    ]);

    const result = await db
      .select()
      .from(users)
      .where(and(eq(users.active, true), gt(users.id, 1)));
    assertEquals(result.length, 1);
    assertEquals(result[0].name, "Carol");
  });
});

Deno.test("drizzle - select with or", async () => {
  await withDb(async (db) => {
    await setupTables(db);
    await db.insert(users).values([
      { id: 1, name: "Alice", email: "alice@example.com" },
      { id: 2, name: "Bob", email: "bob@example.com" },
      { id: 3, name: "Carol", email: "carol@example.com" },
    ]);

    const result = await db
      .select()
      .from(users)
      .where(or(eq(users.id, 1), eq(users.id, 3)));
    assertEquals(result.length, 2);
  });
});

Deno.test("drizzle - select specific columns", async () => {
  await withDb(async (db) => {
    await setupTables(db);
    await db.insert(users).values({
      id: 1,
      name: "Alice",
      email: "alice@example.com",
    });

    const result = await db
      .select({ name: users.name, email: users.email })
      .from(users);
    assertEquals(result.length, 1);
    assertEquals(result[0].name, "Alice");
    assertEquals(result[0].email, "alice@example.com");
    // @ts-expect-error id not selected
    assertEquals(result[0].id, undefined);
  });
});

Deno.test("drizzle - select with order by asc", async () => {
  await withDb(async (db) => {
    await setupTables(db);
    await db.insert(users).values([
      { id: 2, name: "Bob", email: "bob@example.com" },
      { id: 1, name: "Alice", email: "alice@example.com" },
      { id: 3, name: "Carol", email: "carol@example.com" },
    ]);

    const result = await db.select().from(users).orderBy(asc(users.name));
    assertEquals(result[0].name, "Alice");
    assertEquals(result[1].name, "Bob");
    assertEquals(result[2].name, "Carol");
  });
});

Deno.test("drizzle - select with order by desc", async () => {
  await withDb(async (db) => {
    await setupTables(db);
    await db.insert(users).values([
      { id: 1, name: "Alice", email: "alice@example.com" },
      { id: 2, name: "Bob", email: "bob@example.com" },
      { id: 3, name: "Carol", email: "carol@example.com" },
    ]);

    const result = await db.select().from(users).orderBy(desc(users.id));
    assertEquals(result[0].id, 3);
    assertEquals(result[1].id, 2);
    assertEquals(result[2].id, 1);
  });
});

Deno.test("drizzle - select with limit", async () => {
  await withDb(async (db) => {
    await setupTables(db);
    await db.insert(users).values([
      { id: 1, name: "Alice", email: "alice@example.com" },
      { id: 2, name: "Bob", email: "bob@example.com" },
      { id: 3, name: "Carol", email: "carol@example.com" },
    ]);

    const result = await db.select().from(users).orderBy(asc(users.id)).limit(
      2,
    );
    assertEquals(result.length, 2);
    assertEquals(result[0].id, 1);
    assertEquals(result[1].id, 2);
  });
});

Deno.test("drizzle - select with offset", async () => {
  await withDb(async (db) => {
    await setupTables(db);
    await db.insert(users).values([
      { id: 1, name: "Alice", email: "alice@example.com" },
      { id: 2, name: "Bob", email: "bob@example.com" },
      { id: 3, name: "Carol", email: "carol@example.com" },
    ]);

    const result = await db
      .select()
      .from(users)
      .orderBy(asc(users.id))
      .limit(2)
      .offset(1);
    assertEquals(result.length, 2);
    assertEquals(result[0].id, 2);
    assertEquals(result[1].id, 3);
  });
});

// =============================================================================
// UPDATE Tests
// =============================================================================

Deno.test("drizzle - update single field", async () => {
  await withDb(async (db) => {
    await setupTables(db);
    await db.insert(users).values({
      id: 1,
      name: "Alice",
      email: "alice@example.com",
    });

    await db.update(users).set({ name: "Alicia" }).where(eq(users.id, 1));

    const result = await db.select().from(users).where(eq(users.id, 1));
    assertEquals(result[0].name, "Alicia");
  });
});

Deno.test("drizzle - update multiple fields", async () => {
  await withDb(async (db) => {
    await setupTables(db);
    await db.insert(users).values({
      id: 1,
      name: "Alice",
      email: "alice@example.com",
      active: true,
    });

    await db
      .update(users)
      .set({ name: "Alicia", active: false })
      .where(eq(users.id, 1));

    const result = await db.select().from(users).where(eq(users.id, 1));
    assertEquals(result[0].name, "Alicia");
    assertEquals(result[0].active, false);
  });
});

Deno.test("drizzle - update multiple rows", async () => {
  await withDb(async (db) => {
    await setupTables(db);
    await db.insert(users).values([
      { id: 1, name: "Alice", email: "a@example.com", active: false },
      { id: 2, name: "Bob", email: "b@example.com", active: false },
      { id: 3, name: "Carol", email: "c@example.com", active: true },
    ]);

    await db.update(users).set({ active: true }).where(eq(users.active, false));

    const result = await db.select().from(users);
    assertEquals(result.every((u) => u.active === true), true);
  });
});

// =============================================================================
// DELETE Tests
// =============================================================================

Deno.test("drizzle - delete single row", async () => {
  await withDb(async (db) => {
    await setupTables(db);
    await db.insert(users).values([
      { id: 1, name: "Alice", email: "alice@example.com" },
      { id: 2, name: "Bob", email: "bob@example.com" },
    ]);

    await db.delete(users).where(eq(users.id, 1));

    const result = await db.select().from(users);
    assertEquals(result.length, 1);
    assertEquals(result[0].name, "Bob");
  });
});

Deno.test("drizzle - delete multiple rows", async () => {
  await withDb(async (db) => {
    await setupTables(db);
    await db.insert(users).values([
      { id: 1, name: "Alice", email: "a@example.com", active: false },
      { id: 2, name: "Bob", email: "b@example.com", active: false },
      { id: 3, name: "Carol", email: "c@example.com", active: true },
    ]);

    await db.delete(users).where(eq(users.active, false));

    const result = await db.select().from(users);
    assertEquals(result.length, 1);
    assertEquals(result[0].name, "Carol");
  });
});

Deno.test("drizzle - delete all rows", async () => {
  await withDb(async (db) => {
    await setupTables(db);
    await db.insert(users).values([
      { id: 1, name: "Alice", email: "alice@example.com" },
      { id: 2, name: "Bob", email: "bob@example.com" },
    ]);

    await db.delete(users);

    const result = await db.select().from(users);
    assertEquals(result.length, 0);
  });
});

// =============================================================================
// Multi-table Tests
// =============================================================================

Deno.test("drizzle - insert and query related tables", async () => {
  await withDb(async (db) => {
    await setupTables(db);

    await db.insert(users).values({ id: 1, name: "Alice", email: "a@ex.com" });
    await db.insert(posts).values([
      { id: 1, title: "First Post", content: "Hello", author_id: 1 },
      { id: 2, title: "Second Post", content: "World", author_id: 1 },
    ]);

    const userResult = await db.select().from(users);
    assertEquals(userResult.length, 1);

    const postResult = await db
      .select()
      .from(posts)
      .where(eq(posts.author_id, 1));
    assertEquals(postResult.length, 2);
  });
});
