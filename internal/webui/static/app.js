let sessionId = '';
let streaming = false;

async function loadSessions() {
  try {
    const r = await fetch('/api/sessions');
    const sessions = await r.json();
    const list = document.getElementById('sessionList');
    if (!sessions || !sessions.length) {
      list.innerHTML = '<div style="color:var(--text-dim);padding:12px">No sessions</div>';
      return;
    }
    list.innerHTML = sessions.map(s =>
      `<div class="session-item${s.id===sessionId?' active':''}" onclick="selectSession('${s.id}')">
        <div>${s.id}</div>
        <div class="sess-model">${s.model||''}</div>
      </div>`
    ).join('');
  } catch(e) { console.error(e); }
}

async function selectSession(id) {
  sessionId = id;
  document.getElementById('chatMessages').innerHTML = '<div class="welcome">Loading...</div>';
  try {
    const r = await fetch('/api/sessions/'+id);
    const s = await r.json();
    const msgs = document.getElementById('chatMessages');
    msgs.innerHTML = '';
    if (s.entries) {
      for (const e of s.entries) {
        if (e.role === 'user') addMessage('user', extractText(e.content));
        else if (e.role === 'assistant') addMessage('assistant', extractText(e.content));
        else if (e.role === 'tool') addMessage('tool', extractText(e.content));
      }
    }
  } catch(e) { console.error(e); }
  loadSessions();
}

function extractText(content) {
  try {
    if (typeof content === 'string') return content;
    const arr = typeof content==='string' ? JSON.parse(content) : content;
    if (Array.isArray(arr) && arr.length>0) return arr[0].text||'';
  } catch(e) {}
  return String(content||'');
}

function addMessage(role, text) {
  const msgs = document.getElementById('chatMessages');
  const div = document.createElement('div');
  div.className = 'message '+role;
  div.textContent = text;
  msgs.appendChild(div);
  msgs.scrollTop = msgs.scrollHeight;
  return div;
}

async function newSession() {
  try {
    const r = await fetch('/api/sessions', {method:'POST'});
    const s = await r.json();
    sessionId = s.id;
    document.getElementById('chatMessages').innerHTML = '<div class="welcome">New session — Start a conversation</div>';
    loadSessions();
  } catch(e) { console.error(e); }
}

function handleKey(e) {
  if (e.key === 'Enter' && !e.shiftKey) {
    e.preventDefault();
    sendMessage();
  }
}

async function sendMessage() {
  const input = document.getElementById('messageInput');
  const text = input.value.trim();
  if (!text || streaming) return;
  input.value = '';
  streaming = true;
  document.getElementById('sendBtn').disabled = true;
  addMessage('user', text);

  const asstDiv = addMessage('assistant', '');
  asstDiv.classList.add('streaming');
  let fullText = '';

  try {
    const r = await fetch('/api/chat', {
      method: 'POST',
      headers: {'Content-Type': 'application/json'},
      body: JSON.stringify({message:text, session_id:sessionId})
    });

    const reader = r.body.getReader();
    const decoder = new TextDecoder();
    let buffer = '';

    while (true) {
      const {done, value} = await reader.read();
      if (done) break;
      buffer += decoder.decode(value, {stream:true});
      const lines = buffer.split('\n');
      buffer = lines.pop()||'';

      for (const line of lines) {
        if (!line.startsWith('data: ')) continue;
        try {
          const data = JSON.parse(line.slice(6));
          if (data.type === 'session') sessionId = data.id;
          else if (data.type === 'text') { fullText += data.content; asstDiv.textContent = fullText; }
          else if (data.type === 'done') { fullText = data.text||fullText; asstDiv.textContent = fullText; }
          else if (data.type === 'error') { asstDiv.className = 'message error'; asstDiv.textContent = data.content; }
          document.getElementById('chatMessages').scrollTop = document.getElementById('chatMessages').scrollHeight;
        } catch(e) {}
      }
    }
  } catch(e) {
    asstDiv.className = 'message error';
    asstDiv.textContent = 'Error: '+e.message;
  }

  asstDiv.classList.remove('streaming');
  streaming = false;
  document.getElementById('sendBtn').disabled = false;
  loadSessions();
}

function toggleSettings() {
  document.getElementById('settingsModal').classList.toggle('hidden');
}

async function saveSettings() {
  const data = {
    model: document.getElementById('setModel').value,
    base_url: document.getElementById('setBaseURL').value,
    api_key: document.getElementById('setAPIKey').value
  };
  await fetch('/api/settings', {method:'PUT', headers:{'Content-Type':'application/json'}, body:JSON.stringify(data)});
  toggleSettings();
}

loadSessions();
