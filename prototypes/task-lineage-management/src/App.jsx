import { useMemo, useState } from 'react';
import {
  Background,
  BaseEdge,
  Controls,
  Handle,
  Position,
  ReactFlow,
  getBezierPath,
} from '@xyflow/react';
import '@xyflow/react/dist/style.css';
import {
  ArrowSquareOut,
  ArrowsClockwise,
  CaretDown,
  CaretRight,
  ChatCircleDots,
  CheckCircle,
  Circle,
  GitBranch,
  GitCommit,
  Folders,
  MagnifyingGlass,
  PaperPlaneTilt,
  PawPrint,
  Robot,
  Rows,
  ShareNetwork,
  Sparkle,
  UserCircle,
} from '@phosphor-icons/react';

const tasks = [
  { id: 'lineage', title: '构建任务谱系管理', status: '待验收', time: '14:22' },
  { id: 'size-analysis', title: '定位消息响应超限原因', status: '已完成', time: '13:05' },
  { id: 'parser-fix', title: '修复长消息解析边界', status: '待验收', time: '12:18' },
  { id: 'loading', title: '优化会话加载性能', status: '进行中', time: '11:07' },
  { id: 'acceptance', title: '完善任务验收流程', status: '进行中', time: '09:48' },
];

const conversationTree = [
  {
    id: 'main', contextId: 'root', title: '分析消息响应尺寸超限', kind: '主对话', taskIds: ['lineage', 'size-analysis', 'parser-fix'],
    children: [
      { id: 'fork1', contextId: 'fork1', title: '实现任务图', kind: 'Fork' },
      { id: 'child', contextId: 'child', title: '接入 Transcript 扫描', kind: '新对话' },
      { id: 'subagent', contextId: 'subagent', title: '验证状态规则', kind: 'Subagent' },
      { id: 'fork2', contextId: 'fork2', title: '修正并行节点交互', kind: 'Fork' },
    ],
  },
  {
    id: 'loading-main', contextId: 'root', title: '优化会话加载性能', kind: '主对话', taskIds: ['loading'], children: [],
    messages: [['你', '长对话的首次加载需要进一步优化。'], ['Codex', '我会先测量消息读取和渲染阶段的耗时。']],
  },
  {
    id: 'acceptance-main', contextId: 'root', title: '完善任务验收流程', kind: '主对话', taskIds: ['acceptance'], children: [],
    messages: [['你', '任务完成必须由人工确认。'], ['Codex', '我会把自动状态停在待验收，并保留人工完成入口。']],
  },
];

const allConversations = conversationTree.flatMap(root => [root, ...root.children.map(child => ({ ...child, parentId: root.id }))]);
const conversationById = Object.fromEntries(allConversations.map(item => [item.id, item]));

const contextData = {
  root: {
    title: '构建任务谱系管理', kind: '主对话', status: '已结束', thread: 'main', time: '09:10',
    workspace: '/Users/wangxin/codepet', branch: 'main', commit: '未提交', merged: '主工作区',
    messages: [
      ['你', '我们需要管理 Thread 和 Task 的双向关系。'],
      ['Codex', '我会先梳理任务在多个执行上下文中的流转方式。'],
      ['你', '同时保留自由图和时间泳道图。'],
    ],
  },
  fork1: {
    title: '实现任务图', kind: 'Fork', status: '已结束', thread: 'fork-1', time: '10:00',
    workspace: 'worktrees/feat-task-lineage', branch: 'feat/task-lineage-fork-1', commit: 'a83f9c2', merged: '未合入',
    messages: [
      ['Codex', '已建立自由图的分叉和汇合布局。'],
      ['你', '节点 hover 时需要突出完整路径。'],
      ['Codex', '收到，我会补充路径与同线程联动状态。'],
    ],
  },
  child: {
    title: '接入 Transcript 扫描', kind: '新对话', status: '已结束', thread: 'child-1', time: '10:00',
    workspace: 'worktrees/transcript-scan', branch: 'feat/transcript-scan', commit: 'd2c901f', merged: '已合入',
    messages: [
      ['Codex', '开始读取新增 Transcript 记录。'],
      ['你', '只抽取做了什么和任务状态。'],
      ['Codex', '已按增量游标输出任务执行片段。'],
    ],
  },
  subagent: {
    title: '验证状态规则', kind: 'Subagent', status: '已结束', thread: 'sub-1', time: '10:00',
    workspace: '/Users/wangxin/codepet', branch: 'main', commit: '未提交', merged: '主工作区',
    messages: [
      ['Codex', '检查任务三态与线程两态规则。'],
      ['Subagent', '状态规则一致，没有发现冲突。'],
      ['Codex', '结果已返回主对话。'],
    ],
  },
  merge: {
    title: '继续处理', kind: '主对话', status: '已结束', thread: 'main', time: '13:00',
    workspace: '/Users/wangxin/codepet', branch: 'main', commit: '未提交', merged: '主工作区',
    messages: [
      ['Codex', '三个执行上下文都已结束。'],
      ['你', '继续调整节点之间的交互。'],
      ['Codex', '我会回到对应 Fork 继续处理。'],
    ],
  },
  fork2: {
    title: '修正并行节点交互', kind: 'Fork', status: '已结束', thread: 'fork-1', time: '14:00',
    workspace: 'worktrees/feat-task-lineage', branch: 'feat/task-lineage-fork-1', commit: 'a83f9c2', merged: '未合入',
    messages: [
      ['你', '不要推断线程角色，也不要给连线标注意图。'],
      ['Codex', '已改为只显示主对话、Fork、新对话和 Subagent。'],
      ['Codex', '连线现在只表达可观察的方向。'],
      ['Codex', '同一 Thread 的两个节点会同步标记。'],
    ],
  },
  review: {
    title: '继续确认', kind: '主对话', status: '已结束', thread: 'main', time: '15:00',
    workspace: '/Users/wangxin/codepet', branch: 'main', commit: 'a83f9c2', merged: '已合入',
    messages: [
      ['Codex', '已收到 Fork 的最新结果。'],
      ['你', '我会检查交互原型，再决定是否完成任务。'],
      ['Codex', '当前任务保持待验收。'],
    ],
  },
};

const graphPositions = {
  root: { x: 220, y: 10 }, fork1: { x: 0, y: 150 }, child: { x: 220, y: 150 },
  subagent: { x: 440, y: 150 }, merge: { x: 220, y: 290 }, fork2: { x: 90, y: 430 }, review: { x: 350, y: 430 },
};

const lanePositions = {
  root: { x: 140, y: 38 }, merge: { x: 450, y: 38 }, review: { x: 700, y: 38 },
  fork1: { x: 260, y: 178 }, fork2: { x: 620, y: 178 }, child: { x: 260, y: 318 }, subagent: { x: 260, y: 458 },
};

const flowEdges = [
  ['root', 'fork1'], ['root', 'child'], ['root', 'subagent'],
  ['fork1', 'merge'], ['child', 'merge'], ['subagent', 'merge'], ['merge', 'fork2'], ['fork2', 'review'],
];

const taskViews = {
  lineage: { nodes: Object.keys(contextData), edges: flowEdges },
  'size-analysis': {
    nodes: ['root', 'child', 'merge', 'review'],
    edges: [['root', 'child'], ['child', 'merge'], ['merge', 'review']],
    labels: { root: '复现响应超限', child: '扫描大消息记录', merge: '定位序列化边界', review: '确认根因' },
    messages: {
      root: [['你', '长对话中的分析消息出现响应尺寸超限，先定位原因。'], ['Codex', '我会从复现条件、消息体积和序列化边界开始排查。']],
      child: [['Codex', '正在扫描新增执行记录并统计大消息片段。'], ['Codex', '已找到尺寸快速增长的上下文区段。']],
      merge: [['Codex', '多个记录都指向同一处序列化边界。'], ['你', '先确认这是根因，不要扩大修改范围。']],
      review: [['Codex', '根因与复现结果一致。'], ['你', '这个分析任务已确认完成。']],
    },
  },
  'parser-fix': {
    nodes: ['root', 'fork1', 'merge', 'fork2', 'review'],
    edges: [['root', 'fork1'], ['fork1', 'merge'], ['merge', 'fork2'], ['fork2', 'review']],
    labels: { root: '确认修复范围', fork1: '修改流式解析', merge: '汇总实现结果', fork2: '补充边界处理', review: '等待人工验收' },
    messages: {
      root: [['你', '按已经确认的根因修复，但只处理长消息边界。'], ['Codex', '修复范围已限定在流式解析与尺寸保护。']],
      fork1: [['Codex', '已在 Fork 中修改流式解析逻辑。'], ['Codex', '代码已提交，等待返回主对话。']],
      merge: [['Codex', '实现结果已汇总，准备补充边界场景。']],
      fork2: [['你', '再补一个临界尺寸下的处理。'], ['Codex', '边界处理已补充并提交。']],
      review: [['Codex', '所有执行线程均已结束，当前任务待验收。']],
    },
  },
};

function Status({ children, type = 'task' }) {
  return <span className={`status status-${type} status-${children}`}>{children}</span>;
}

function ContextIcon({ kind }) {
  if (kind === '主对话') return <UserCircle weight="duotone" />;
  if (kind === 'Subagent') return <Robot weight="duotone" />;
  if (kind === '新对话') return <ChatCircleDots weight="duotone" />;
  return <GitBranch weight="duotone" />;
}

function ExecutionNode({ data }) {
  const { item, active, related, dimmed, onEnter, onLeave, onSelect, view } = data;
  const committed = item.commit !== '未提交';
  const inWorktree = item.workspace.includes('worktrees/');
  const syncState = inWorktree ? (item.merged === '已合入' ? '已同步' : '未同步') : '无需同步';
  return (
    <button
      className={`execution-node ${active ? 'active' : ''} ${related ? 'same-thread' : ''} ${dimmed ? 'dimmed' : ''}`}
      onMouseEnter={onEnter}
      onMouseLeave={onLeave}
      onFocus={onEnter}
      onBlur={onLeave}
      onClick={onSelect}
      aria-label={`${item.title}，${item.kind}，${item.status}，${inWorktree ? '独立 worktree' : '主工作区'}，${committed ? '已提交' : '未提交'}，${syncState}`}
    >
      <Handle type="target" position={view === 'graph' ? Position.Top : Position.Left} className="flow-handle" />
      <span className="node-icon"><ContextIcon kind={item.kind} /></span>
      <span className="node-copy">
        <strong>{item.title}</strong>
        <small>{item.kind}</small>
      </span>
      <span className="node-meta"><time>{item.time}</time><Status type="thread">{item.status}</Status></span>
      <span className="node-code-state">
        <span><Folders weight="bold" />{inWorktree ? '独立 worktree' : '主工作区'}</span>
        <span className={committed ? 'positive' : 'attention'}><GitCommit weight="bold" />{committed ? '已提交' : '未提交'}</span>
        <span className={syncState === '已同步' ? 'positive' : syncState === '未同步' ? 'attention' : ''}><ArrowsClockwise weight="bold" />{syncState}</span>
      </span>
      <Handle type="source" position={view === 'graph' ? Position.Bottom : Position.Right} className="flow-handle" />
    </button>
  );
}

function FlowEdge({ id, sourceX, sourceY, targetX, targetY, sourcePosition, targetPosition, data, markerEnd }) {
  const [path] = getBezierPath({ sourceX, sourceY, targetX, targetY, sourcePosition, targetPosition, curvature: .22 });
  return <BaseEdge id={id} path={path} markerEnd={markerEnd} className={`custom-edge ${data?.active ? 'active' : ''} ${data?.dimmed ? 'dimmed' : ''}`} />;
}

function LaneNode({ data }) {
  return <div className="lane-node"><strong>{data.label}</strong>{data.times && <div className="lane-times">{data.times.map(time => <span key={time}>{time}</span>)}</div>}</div>;
}

const nodeTypes = { execution: ExecutionNode, lane: LaneNode };
const edgeTypes = { flow: FlowEdge };

function getRouteSet(hovered, edges) {
  if (!hovered) return new Set();
  const parents = new Map();
  const children = new Map();
  edges.forEach(([source, target]) => {
    parents.set(target, [...(parents.get(target) || []), source]);
    children.set(source, [...(children.get(source) || []), target]);
  });
  const result = new Set([hovered]);
  const walk = (id, map) => (map.get(id) || []).forEach(next => {
    if (!result.has(next)) { result.add(next); walk(next, map); }
  });
  walk(hovered, parents);
  walk(hovered, children);
  return result;
}

function TaskFlow({ view, taskId, hovered, setHovered, selected, setSelected, runtimeStatuses }) {
  const taskView = taskViews[taskId] || taskViews.lineage;
  const route = useMemo(() => getRouteSet(hovered, taskView.edges), [hovered, taskView]);
  const selectedThread = contextData[hovered || selected].thread;
  const positions = view === 'graph' ? graphPositions : lanePositions;
  const executionNodes = taskView.nodes.map(id => {
    const baseItem = contextData[id];
    const item = { ...baseItem, title: taskView.labels?.[id] || baseItem.title, status: runtimeStatuses[id] };
    return ({
    id,
    type: 'execution',
    position: positions[id],
    sourcePosition: view === 'graph' ? Position.Bottom : Position.Right,
    targetPosition: view === 'graph' ? Position.Top : Position.Left,
    data: {
      item,
      view,
      active: id === (hovered || selected),
      related: item.thread === selectedThread && id !== (hovered || selected),
      dimmed: Boolean(hovered && !route.has(id)),
      onEnter: () => setHovered(id),
      onLeave: () => setHovered(null),
      onSelect: () => setSelected(id),
    },
    });
  });
  const laneNodes = view === 'swimlane' ? ['主对话 · Codex', 'Fork · Codex', '新对话 · Codex', 'Subagent'].map((label, index) => ({
    id: `lane-${index}`,
    type: 'lane',
    position: { x: 0, y: index * 140 },
    data: { label, times: index === 0 ? ['09:00', '11:00', '13:00', '15:00', '17:00'] : null },
    selectable: false,
    draggable: false,
    focusable: false,
    zIndex: -1,
    style: { width: 900, height: 140 },
  })) : [];
  const nodes = [...laneNodes, ...executionNodes];
  const edges = taskView.edges.map(([source, target], index) => ({
    id: `e${index}`,
    source, target, type: 'flow',
    markerEnd: { type: 'arrowclosed', color: hovered && route.has(source) && route.has(target) ? '#1769e8' : '#aab5c8', width: 16, height: 16 },
    data: {
      active: Boolean(hovered && route.has(source) && route.has(target)),
      dimmed: Boolean(hovered && !(route.has(source) && route.has(target))),
    },
  }));

  return (
    <div className={`flow-shell ${view}`}>
      <ReactFlow
        key={view}
        nodes={nodes}
        edges={edges}
        nodeTypes={nodeTypes}
        edgeTypes={edgeTypes}
        fitView={view === 'graph'}
        defaultViewport={view === 'swimlane' ? { x: 24, y: 118, zoom: .82 } : { x: 0, y: 0, zoom: 1 }}
        fitViewOptions={{ padding: view === 'graph' ? .15 : .04 }}
        minZoom={.55}
        maxZoom={1.45}
        nodesDraggable={view === 'graph'}
        nodesConnectable={false}
        elementsSelectable
        onNodeClick={(_, node) => node.type === 'execution' && setSelected(node.id)}
      >
        {view === 'graph' && <Background color="#e9edf4" gap={24} size={1} />}
        <Controls showInteractive={false} position="bottom-right" />
      </ReactFlow>
      <div className="legend"><span><i className="line-key" />当前流向</span><span><i className="thread-key" />同一线程</span></div>
    </div>
  );
}

function Sidebar({ listMode, setListMode, currentTaskStatus, selectedListId, onSelect }) {
  const [expanded, setExpanded] = useState(() => new Set(['main']));
  const toggleRoot = id => setExpanded(current => {
    const next = new Set(current);
    if (next.has(id)) next.delete(id);
    else next.add(id);
    return next;
  });
  return (
    <aside className="sidebar">
      <div className="brand"><PawPrint weight="fill" /><strong>CodePet</strong></div>
      <h2>任务管理</h2>
      <label className="search"><MagnifyingGlass /><input placeholder={listMode === 'tasks' ? '搜索任务…' : '搜索对话…'} /></label>
      <div className="segmented wide">
        <button className={listMode === 'threads' ? 'selected' : ''} onClick={() => setListMode('threads')}>对话</button>
        <button className={listMode === 'tasks' ? 'selected' : ''} onClick={() => setListMode('tasks')}>任务</button>
      </div>
      <nav className="list">
        {listMode === 'tasks' ? tasks.map(baseItem => {
          const item = baseItem.id === 'lineage' ? { ...baseItem, status: currentTaskStatus } : baseItem;
          return <button className={item.id === selectedListId ? 'list-row selected-row' : 'list-row'} key={item.id} onClick={() => onSelect(item.id)}>
            <span className="row-title"><Circle weight="fill" /><strong>{item.title}</strong></span>
            <Status>{item.status}</Status><small>{item.time}</small>
          </button>;
        }) : conversationTree.map(root => (
          <div className="conversation-branch" key={root.id}>
            <div className={`conversation-row ${root.id === selectedListId ? 'selected-row' : ''}`}>
              <button className="expand-button" onClick={() => toggleRoot(root.id)} aria-label={`${expanded.has(root.id) ? '收起' : '展开'} ${root.title}`}>
                {expanded.has(root.id) ? <CaretDown weight="bold" /> : <CaretRight weight="bold" />}
              </button>
              <button className="conversation-select" onClick={() => onSelect(root.id)}>
                <span className="row-title"><ContextIcon kind="主对话" /><strong>{root.title}</strong></span>
                <small>主对话</small>
              </button>
              <span className="task-count">{root.taskIds.length} 个任务</span>
            </div>
            {expanded.has(root.id) && root.children.length > 0 && <div className="child-list">
              {root.children.map(child => <button className={`child-row ${child.id === selectedListId ? 'selected-row' : ''}`} key={child.id} onClick={() => onSelect(child.id)}>
                <span className="child-rail" />
                <span className="child-icon"><ContextIcon kind={child.kind} /></span>
                <span className="child-copy"><strong>{child.title}</strong><small>{child.kind}</small></span>
              </button>)}
            </div>}
          </div>
        ))}
      </nav>
      <div className="list-count">{listMode === 'tasks' ? `共 ${tasks.length} 项任务` : `codepet · ${conversationTree.length} 个主对话`}</div>
    </aside>
  );
}

function MessageList({ storeKey, defaultMessages, time, messages, setMessages, onSend, surface = 'inspector' }) {
  const [draft, setDraft] = useState('');
  const shown = messages[storeKey] || defaultMessages;
  const send = () => {
    if (!draft.trim()) return;
    setMessages(previous => ({ ...previous, [storeKey]: [...shown, ['你', draft.trim()]] }));
    onSend?.();
    setDraft('');
  };
  return <div className={`message-list message-list-${surface}`}>
    <div className="messages">
      {shown.map(([author, body], index) => (
        <article className={author === '你' ? 'message human' : 'message agent'} key={`${author}-${index}`}>
          <span className="avatar">{author === '你' ? <UserCircle /> : <Sparkle weight="fill" />}</span>
          <div><header><strong>{author}</strong><time>{time}</time></header><p>{body}</p></div>
        </article>
      ))}
    </div>
    <div className="composer">
      <textarea value={draft} onChange={event => setDraft(event.target.value)} onKeyDown={event => {
        if (event.key === 'Enter' && !event.shiftKey) { event.preventDefault(); send(); }
      }} placeholder="输入消息，按 Enter 发送…" aria-label="发送到当前对话" />
      <button onClick={send} aria-label="发送消息"><PaperPlaneTilt weight="fill" /></button>
    </div>
  </div>;
}

function Inspector({ selected, taskId, messages, setMessages, runtimeStatuses, onRun }) {
  const taskView = taskViews[taskId] || taskViews.lineage;
  const baseItem = contextData[selected];
  const item = { ...baseItem, title: taskView.labels?.[selected] || baseItem.title, status: runtimeStatuses[selected] };
  const messageKey = `${taskId}:${selected}`;
  const defaultMessages = taskView.messages?.[selected] || item.messages;
  const inWorktree = item.workspace.includes('worktrees/');
  const syncState = inWorktree ? (item.merged === '已合入' ? '已同步' : '未同步') : '无需同步';
  return (
    <aside className="inspector">
      <header className="inspect-head">
        <div><span className="inspect-kind"><ContextIcon kind={item.kind} /> {item.kind}</span><h2>{item.title}</h2><Status type="thread">{item.status}</Status></div>
        <button className="outline-button" onClick={() => window.alert(`将打开：${item.title}`)}>打开原始对话 <ArrowSquareOut /></button>
      </header>
      <dl className="facts">
        <div><dt>工作模式</dt><dd>{inWorktree ? '独立 worktree' : '主工作区'}</dd></div>
        <div><dt>工作区</dt><dd>{item.workspace}</dd></div>
        <div><dt>分支</dt><dd>{item.branch}</dd></div>
        <div><dt>提交</dt><dd>{item.commit}</dd></div>
        <div><dt>同步到主工作区</dt><dd className={syncState !== '未同步' ? 'ok' : ''}>{syncState}</dd></div>
      </dl>
      <MessageList key={messageKey} storeKey={messageKey} defaultMessages={defaultMessages} time={item.time} messages={messages} setMessages={setMessages} onSend={() => onRun(selected)} />
    </aside>
  );
}

function ConversationView({ conversation, messages, setMessages, onRun, runtimeStatuses }) {
  const item = contextData[conversation.contextId];
  const defaultMessages = conversation.messages || item.messages;
  return <section className="conversation-view">
    <header className="conversation-heading">
      <span className="conversation-avatar"><ContextIcon kind={conversation.kind} /></span>
      <div><span>{conversation.kind}</span><h1>{conversation.title}</h1></div>
      <Status type="thread">{runtimeStatuses[conversation.contextId]}</Status>
    </header>
    <MessageList key={conversation.id} surface="center" storeKey={`conversation:${conversation.id}`} defaultMessages={defaultMessages} time={item.time} messages={messages} setMessages={setMessages} onSend={() => onRun(conversation.contextId)} />
  </section>;
}

function ConversationInfo({ conversation }) {
  const item = contextData[conversation.contextId];
  const parent = conversation.parentId ? conversationById[conversation.parentId] : null;
  const taskIds = conversation.taskIds || parent?.taskIds || [];
  return <aside className="inspector conversation-info">
    <header className="inspect-head">
      <span className="inspect-kind"><ContextIcon kind={conversation.kind} /> {conversation.kind}</span>
      <h2>对话信息</h2>
    </header>
    <dl className="facts">
      <div><dt>项目</dt><dd>codepet</dd></div>
      <div><dt>标题</dt><dd>{conversation.title}</dd></div>
      {parent && <div><dt>所属主对话</dt><dd>{parent.title}</dd></div>}
      <div><dt>工作区</dt><dd>{item.workspace}</dd></div>
      <div><dt>分支</dt><dd>{item.branch}</dd></div>
    </dl>
    <section className="related-tasks">
      <h3>关联任务 <span>{taskIds.length}</span></h3>
      {taskIds.map(taskId => {
        const task = tasks.find(candidate => candidate.id === taskId);
        return task && <div className="related-task" key={task.id}><strong>{task.title}</strong><Status>{task.status}</Status></div>;
      })}
    </section>
  </aside>;
}

export function App() {
  const [listMode, setListMode] = useState('threads');
  const [selectedListId, setSelectedListId] = useState('main');
  const [centerMode, setCenterMode] = useState('conversation');
  const [activeTask, setActiveTask] = useState('lineage');
  const [view, setView] = useState('graph');
  const [hovered, setHovered] = useState(null);
  const [selected, setSelected] = useState('fork2');
  const [messages, setMessages] = useState({});
  const [runtimeStatuses, setRuntimeStatuses] = useState(() => Object.fromEntries(Object.keys(contextData).map(id => [id, '已结束'])));
  const [manualComplete, setManualComplete] = useState(false);
  const currentTaskStatus = manualComplete ? '已完成' : Object.values(runtimeStatuses).includes('运行中') ? '进行中' : '待验收';
  const selectedConversation = conversationById[selectedListId] || conversationTree[0];
  const activeTaskData = tasks.find(task => task.id === activeTask) || tasks[0];
  const activeTaskStatus = activeTask === 'lineage' ? currentTaskStatus : activeTaskData.status;
  const visibleTaskIds = listMode === 'threads' ? (selectedConversation.taskIds || []) : [activeTask];
  const chooseTask = taskId => {
    setActiveTask(taskId);
    setCenterMode('task');
    const firstNode = (taskViews[taskId] || taskViews.lineage).nodes[0];
    setSelected(firstNode);
    setHovered(null);
  };
  const chooseListItem = id => {
    setSelectedListId(id);
    if (listMode === 'threads') {
      setCenterMode('conversation');
      const conversation = conversationById[id];
      if (conversation?.taskIds?.length) setActiveTask(conversation.taskIds[0]);
    } else chooseTask(id);
  };
  const changeListMode = mode => {
    setListMode(mode);
    if (mode === 'threads') {
      setSelectedListId('main');
      setCenterMode('conversation');
    } else {
      setSelectedListId(activeTask);
      setCenterMode('task');
    }
  };
  const startRun = id => {
    setManualComplete(false);
    setRuntimeStatuses(statuses => ({ ...statuses, [id]: '运行中' }));
  };

  return (
    <main className="app-shell">
      <Sidebar listMode={listMode} setListMode={changeListMode} currentTaskStatus={currentTaskStatus} selectedListId={selectedListId} onSelect={chooseListItem} />
      <section className="workspace">
        <header className="workspace-top">
          {listMode === 'threads' ? <div className="segmented view-tabs">
            <button className={centerMode === 'conversation' ? 'selected' : ''} onClick={() => setCenterMode('conversation')}>对话</button>
            {selectedConversation.kind === '主对话' && <button className={centerMode === 'task' ? 'selected' : ''} onClick={() => setCenterMode('task')}>任务</button>}
          </div> : <strong className="section-label">任务详情</strong>}
          <span className="project-label"><Folders weight="duotone" /> codepet</span>
        </header>
        {centerMode === 'task' ? <>
          <div className="task-toolbar">
            <div className="task-tabs" aria-label="当前对话中的任务">
              {visibleTaskIds.map(taskId => {
                const task = tasks.find(item => item.id === taskId) || tasks[0];
                return <button key={taskId} className={activeTask === taskId ? 'selected' : ''} onClick={() => chooseTask(taskId)}>{task.title}<Status>{taskId === 'lineage' ? currentTaskStatus : task.status}</Status></button>;
              })}
            </div>
            <div className="segmented compact">
              <button className={view === 'graph' ? 'selected' : ''} onClick={() => setView('graph')}><ShareNetwork /> 图状</button>
              <button className={view === 'swimlane' ? 'selected' : ''} onClick={() => setView('swimlane')}><Rows /> 泳道</button>
            </div>
          </div>
          <div className="task-heading"><div><h1>{activeTaskData.title} <Status>{activeTaskStatus}</Status></h1><p>从对话记录中还原任务跨执行上下文的客观流向。</p></div>{activeTask === 'lineage' && activeTaskStatus === '待验收' && <button className="complete-button" onClick={() => setManualComplete(true)}><CheckCircle /> 标记已完成</button>}</div>
          <TaskFlow view={view} taskId={activeTask} hovered={hovered} setHovered={setHovered} selected={selected} setSelected={setSelected} runtimeStatuses={runtimeStatuses} />
        </> : <ConversationView conversation={selectedConversation} messages={messages} setMessages={setMessages} onRun={startRun} runtimeStatuses={runtimeStatuses} />}
      </section>
      {centerMode === 'task'
        ? <Inspector selected={selected} taskId={activeTask} messages={messages} setMessages={setMessages} runtimeStatuses={runtimeStatuses} onRun={startRun} />
        : <ConversationInfo conversation={selectedConversation} />}
    </main>
  );
}
