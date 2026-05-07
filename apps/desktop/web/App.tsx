import { useState } from 'react';
import { PanelLeftClose, Plus, MessageSquare, MoreHorizontal, LayoutDashboard, Terminal, FileText, Brain, ListChecks, Target, BookOpen } from 'lucide-react';
import { Button } from '@/components/ui/button';
import { cn } from '@/lib/utils';

const MOCK_TASKS = [
  { 
    id: 1, 
    title: 'Initialize Project', 
    status: 'Completed', 
    time: '10m ago',
    agents: '# Project Setup Agent\nResponsible for scaffolding...',
    memories: ['Last run: Success', 'Node version: 20.x'],
    spec: '## Specification\nProject should use Vite + React...',
    plan: '1. Init git\n2. Install deps\n3. Setup CI',
    task: 'Current task: Finalize setup'
  },
  { 
    id: 2, 
    title: 'Design System Update', 
    status: 'In Progress', 
    time: '2m ago',
    agents: '# UI/UX Agent\nUpdating tokens and components...',
    memories: ['Design system: Radix', 'Colors: OKLCH'],
    spec: '## UI Spec\nImplement dark mode support...',
    plan: '1. Audit colors\n2. Update themes',
    task: 'Current task: Color palette'
  },
  { id: 3, title: 'Implement Auth Flow', status: 'Pending', time: '1h ago' },
  { id: 4, title: 'Database Schema Design', status: 'Pending', time: '3h ago' },
  { id: 5, title: 'Bug Fix: Sidebar Toggle', status: 'Completed', time: '5h ago' },
];

type ViewMode = 'home' | 'terminal';

function App() {
  const [activeTaskId, setActiveTaskId] = useState<number | null>(1);
  const [viewMode, setViewMode] = useState<ViewMode>('home');

  const activeTask = MOCK_TASKS.find(t => t.id === activeTaskId);

  return (
    <div className="flex h-screen w-full bg-background text-foreground overflow-hidden font-sans">
      {/* Side Panel */}
      <aside className="w-[280px] flex flex-col border-r bg-sidebar text-sidebar-foreground shrink-0">
        {/* Side Panel Header */}
        <header className="h-12 flex items-center justify-between px-4 border-b border-sidebar-border/50">
          <div className="flex items-center gap-2">
            <div className="w-5 h-5 bg-black rounded flex items-center justify-center text-[9px] text-white font-bold">
              O
            </div>
            <h1 className="font-semibold text-xs tracking-tight">Ora Desktop</h1>
          </div>
          <Button variant="ghost" size="icon" className="h-7 w-7 text-sidebar-foreground/70 hover:text-sidebar-foreground">
            <PanelLeftClose className="h-3.5 w-3.5" />
          </Button>
        </header>

        {/* Side Panel Content - Agent Tasks */}
        <div className="flex-1 overflow-y-auto py-3 px-2 space-y-4">
          <div className="px-2">
            <div className="flex items-center justify-between mb-2">
              <h2 className="text-[10px] font-semibold uppercase tracking-wider text-sidebar-foreground/50">
                Agent Tasks
              </h2>
              <Button variant="ghost" size="icon" className="h-4 w-4 text-sidebar-foreground/50 hover:text-sidebar-foreground">
                <Plus className="h-3 w-3" />
              </Button>
            </div>
            
            <div className="space-y-1">
              {MOCK_TASKS.map((task) => (
                <div
                  key={task.id}
                  onClick={() => setActiveTaskId(task.id)}
                  className={cn(
                    "group flex flex-col gap-1 p-2 rounded-md cursor-pointer transition-colors",
                    activeTaskId === task.id 
                      ? "bg-sidebar-accent text-sidebar-accent-foreground" 
                      : "hover:bg-sidebar-accent/50 hover:text-sidebar-accent-foreground text-sidebar-foreground/80"
                  )}
                >
                  <div className="flex items-start justify-between">
                    <div className="flex items-center gap-2 min-w-0">
                      <MessageSquare className="h-3.5 w-3.5 shrink-0 text-sidebar-foreground/60" />
                      <span className="text-sm font-medium truncate">{task.title}</span>
                    </div>
                    <Button variant="ghost" size="icon" className="h-4 w-4 opacity-0 group-hover:opacity-100 transition-opacity">
                      <MoreHorizontal className="h-3 w-3" />
                    </Button>
                  </div>
                  <div className="flex items-center gap-2 pl-5.5">
                    <span className={cn(
                      "text-[10px] px-1.5 py-0.5 rounded-full",
                      task.status === 'Completed' ? 'bg-green-500/10 text-green-600' :
                      task.status === 'In Progress' ? 'bg-blue-500/10 text-blue-600' :
                      'bg-orange-500/10 text-orange-600'
                    )}>
                      {task.status}
                    </span>
                    <span className="text-[10px] text-sidebar-foreground/40">{task.time}</span>
                  </div>
                </div>
              ))}
            </div>
          </div>
        </div>

        {/* Side Panel Footer */}
        <footer className="p-3 border-t border-sidebar-border/50">
          <div className="flex items-center gap-2.5 p-1.5 rounded-lg hover:bg-sidebar-accent cursor-pointer transition-colors">
            <div className="w-7 h-7 rounded-full bg-muted flex items-center justify-center text-[10px] font-medium border border-sidebar-border">
              JD
            </div>
            <div className="flex-1 min-w-0">
              <p className="text-xs font-medium truncate">John Doe</p>
              <p className="text-[10px] text-sidebar-foreground/50 truncate">john@example.com</p>
            </div>
          </div>
        </footer>
      </aside>

      {/* Main Content Area */}
      <main className="flex-1 flex flex-col min-w-0 bg-background overflow-hidden">
        <header className="h-12 border-b flex items-center px-4 shrink-0 bg-background/50 backdrop-blur justify-between">
          <div className="flex items-center gap-4 h-full">
            <h2 className="text-xs font-semibold truncate text-muted-foreground border-r pr-4 mr-1 py-1">
              {activeTask ? activeTask.title : 'Dashboard'}
            </h2>
            
            {activeTask && (
              <nav className="flex h-full items-center gap-1">
                <button
                  onClick={() => setViewMode('home')}
                  className={cn(
                    "px-3 h-8 rounded-md text-xs font-medium transition-all flex items-center gap-1.5",
                    viewMode === 'home' 
                      ? "bg-secondary text-secondary-foreground shadow-sm" 
                      : "text-muted-foreground hover:bg-muted hover:text-foreground"
                  )}
                >
                  <LayoutDashboard className="h-3.5 w-3.5" />
                  Home
                </button>
                <button
                  onClick={() => setViewMode('terminal')}
                  className={cn(
                    "px-3 h-8 rounded-md text-xs font-medium transition-all flex items-center gap-1.5",
                    viewMode === 'terminal' 
                      ? "bg-secondary text-secondary-foreground shadow-sm" 
                      : "text-muted-foreground hover:bg-muted hover:text-foreground"
                  )}
                >
                  <Terminal className="h-3.5 w-3.5" />
                  Terminal
                </button>
              </nav>
            )}
          </div>

          <div className="flex items-center gap-2">
            <Button variant="ghost" size="icon" className="h-8 w-8 text-muted-foreground">
              <MoreHorizontal className="h-4 w-4" />
            </Button>
          </div>
        </header>
        
        <div className="flex-1 overflow-y-auto bg-muted/20">
          {activeTask ? (
            viewMode === 'home' ? (
              /* Task Home View */
              <div className="p-6 grid grid-cols-1 md:grid-cols-2 gap-4 max-w-6xl mx-auto">
                {/* AGENTS.md */}
                <div className="p-4 rounded-xl border bg-card shadow-sm col-span-full">
                  <div className="flex items-center gap-2 mb-3 border-b pb-2">
                    <BookOpen className="h-4 w-4 text-primary" />
                    <h3 className="text-xs font-semibold uppercase tracking-wider text-muted-foreground">AGENTS.md</h3>
                  </div>
                  <pre className="text-sm text-card-foreground/80 font-mono whitespace-pre-wrap">
                    {activeTask.agents || 'No agent info available.'}
                  </pre>
                </div>

                {/* Memory */}
                <div className="p-4 rounded-xl border bg-card shadow-sm">
                  <div className="flex items-center gap-2 mb-3 border-b pb-2">
                    <Brain className="h-4 w-4 text-purple-500" />
                    <h3 className="text-xs font-semibold uppercase tracking-wider text-muted-foreground">Memory</h3>
                  </div>
                  <div className="space-y-2">
                    {activeTask.memories?.map((m, i) => (
                      <div key={i} className="text-sm p-2 rounded bg-muted/50 border border-border/50">
                        {m}
                      </div>
                    )) || <p className="text-sm text-muted-foreground">Empty memory.</p>}
                  </div>
                </div>

                {/* Spec.md */}
                <div className="p-4 rounded-xl border bg-card shadow-sm">
                  <div className="flex items-center gap-2 mb-3 border-b pb-2">
                    <Target className="h-4 w-4 text-blue-500" />
                    <h3 className="text-xs font-semibold uppercase tracking-wider text-muted-foreground">Spec.md</h3>
                  </div>
                  <p className="text-sm text-card-foreground/70 line-clamp-4">
                    {activeTask.spec || 'No specification provided.'}
                  </p>
                </div>

                {/* Plan.md */}
                <div className="p-4 rounded-xl border bg-card shadow-sm">
                  <div className="flex items-center gap-2 mb-3 border-b pb-2">
                    <ListChecks className="h-4 w-4 text-orange-500" />
                    <h3 className="text-xs font-semibold uppercase tracking-wider text-muted-foreground">Plan.md</h3>
                  </div>
                  <pre className="text-sm text-card-foreground/70 font-mono whitespace-pre-wrap">
                    {activeTask.plan || 'No plan defined.'}
                  </pre>
                </div>

                {/* Task.md */}
                <div className="p-4 rounded-xl border bg-card shadow-sm">
                  <div className="flex items-center gap-2 mb-3 border-b pb-2">
                    <FileText className="h-4 w-4 text-green-500" />
                    <h3 className="text-xs font-semibold uppercase tracking-wider text-muted-foreground">Task.md</h3>
                  </div>
                  <p className="text-sm text-card-foreground/70 italic">
                    {activeTask.task || 'No active task description.'}
                  </p>
                </div>
              </div>
            ) : (
              /* Terminal View */
              <div className="h-full bg-black text-green-500 font-mono p-4 overflow-hidden flex flex-col">
                <div className="flex-1 overflow-y-auto">
                  <p className="mb-2 text-white/50"># Terminal Session - {activeTask.title}</p>
                  <p className="mb-1 text-blue-400">➜  ora-desktop git:(main) ✗ <span className="text-white">npm run dev</span></p>
                  <p className="mb-1">VITE v8.0.10  ready in 128 ms</p>
                  <p className="mb-1">  ➜  Local:   http://localhost:5173/</p>
                  <p className="mb-1">  ➜  Network: use --host to expose</p>
                  <p className="animate-pulse inline-block w-2 h-4 bg-green-500 align-middle ml-1" />
                </div>
              </div>
            )
          ) : (
            /* Empty Dashboard */
            <div className="h-full flex flex-col items-center justify-center text-center p-8">
              <div className="w-12 h-12 rounded-full bg-muted flex items-center justify-center mb-4">
                <Plus className="h-6 w-6 text-muted-foreground" />
              </div>
              <h3 className="text-lg font-semibold mb-1">No active workspace selected</h3>
              <p className="text-sm text-muted-foreground max-w-[280px]">
                Choose an agent task from the sidebar or create a new one to get started.
              </p>
              <Button className="mt-6">Create New Task</Button>
            </div>
          )}
        </div>
      </main>
    </div>
  );
}

export default App;
