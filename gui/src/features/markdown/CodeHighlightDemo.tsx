import { MarkdownContent } from "./MarkdownContent";

const demoMarkdown = `
# Code Highlighting Demo

## TypeScript

\`\`\`typescript
interface User {
  id: number;
  name: string;
  email: string;
}

function greet(user: User): string {
  return \`Hello, \${user.name}!\`;
}

const users: User[] = [
  { id: 1, name: "Alice", email: "alice@example.com" },
  { id: 2, name: "Bob", email: "bob@example.com" },
];

users.forEach(user => {
  console.log(greet(user));
});
\`\`\`

## Rust

\`\`\`rust
use std::collections::HashMap;

struct Config {
    name: String,
    values: HashMap<String, i32>,
}

impl Config {
    fn new(name: &str) -> Self {
        Config {
            name: name.to_string(),
            values: HashMap::new(),
        }
    }

    fn add_value(&mut self, key: &str, value: i32) {
        self.values.insert(key.to_string(), value);
    }
}

fn main() {
    let mut config = Config::new("test");
    config.add_value("timeout", 30);
    println!("Config: {}", config.name);
}
\`\`\`

## Python

\`\`\`python
from dataclasses import dataclass
from typing import List, Optional

@dataclass
class Task:
    id: int
    title: str
    completed: bool = False

class TaskManager:
    def __init__(self):
        self.tasks: List[Task] = []
    
    def add_task(self, title: str) -> Task:
        task = Task(id=len(self.tasks) + 1, title=title)
        self.tasks.append(task)
        return task
    
    def complete_task(self, task_id: int) -> Optional[Task]:
        for task in self.tasks:
            if task.id == task_id:
                task.completed = True
                return task
        return None

manager = TaskManager()
task = manager.add_task("Learn Rust")
manager.complete_task(task.id)
\`\`\`

## JSON

\`\`\`json
{
  "name": "future-os",
  "version": "1.0.0",
  "dependencies": {
    "react": "^18.3.1",
    "typescript": "^5.7.2"
  },
  "scripts": {
    "dev": "vite",
    "build": "tsc && vite build"
  }
}
\`\`\`

## Shell

\`\`\`bash
#!/bin/bash

# Build the project
echo "Building FutureOS..."
cd gui || exit 1

npm install
npm run build

if [ $? -eq 0 ]; then
    echo "Build successful!"
else
    echo "Build failed!"
    exit 1
fi
\`\`\`

## SQL

\`\`\`sql
SELECT 
    u.id,
    u.name,
    COUNT(o.id) as order_count,
    SUM(o.total) as total_spent
FROM users u
LEFT JOIN orders o ON u.id = o.user_id
WHERE u.created_at >= '2024-01-01'
GROUP BY u.id, u.name
HAVING COUNT(o.id) > 5
ORDER BY total_spent DESC
LIMIT 10;
\`\`\`
`;

export function CodeHighlightDemo() {
  return (
    <div className="mx-auto max-w-4xl p-8">
      <MarkdownContent content={demoMarkdown} />
    </div>
  );
}
