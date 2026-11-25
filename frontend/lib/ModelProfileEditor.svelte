<script lang="ts">
  import { onMount, onDestroy, tick, type Snippet } from "svelte";
  import { RefreshCw, Eye, EyeOff, LoaderCircle } from "@lucide/svelte";
  import {
    modelCatalogRequest,
    type AvailableModel,
    type ModelProfile,
  } from "./modelGateway";
  import {
    modelProviders,
    providerId,
    sameEndpoint,
    isLocalEndpoint,
  } from "./modelProviders";
  export let purpose: "llm" | "embedding";
  export let profile: ModelProfile;
  export let savedProfile: ModelProfile;
  export let enabled = false;
  export let advanced: Snippet | undefined = undefined;
  let models: AvailableModel[] = [],
    loading = false,
    error = "",
    manual = false,
    showKey = false,
    advancedOpen = false;
  let generation = 0,
    disposed = false,
    modelInput: HTMLInputElement;
  let lastEndpoint = profile.endpoint,
    lastModel = profile.model,
    customProvider = false;
  $: label = purpose === "llm" ? "语言模型" : "向量模型";
  $: selectedProvider = customProvider
    ? "custom"
    : providerId(profile.endpoint);
  $: storedKey =
    !!savedProfile.hasApiKey &&
    sameEndpoint(savedProfile.endpoint, profile.endpoint);
  $: canDetect = !!profile.endpoint;
  $: visibleModels = models.filter(
    (model) =>
      !model.kind ||
      model.kind === (purpose === "llm" ? "llm" : "embedding") ||
      (purpose === "llm" && model.kind === "language"),
  );
  export function showAdvanced() {
    advancedOpen = true;
  }
  function autoDetect() {
    if (isLocalEndpoint(profile.endpoint) || profile.apiKey || storedKey)
      void detect();
  }
  function clearModels() {
    generation++;
    models = [];
    error = "";
    loading = false;
  }
  async function keyChanged() {
    clearModels();
    await tick();
    autoDetect();
  }
  async function detect() {
    if (!canDetect || loading) return;
    const request = ++generation,
      endpoint = profile.endpoint,
      key = profile.apiKey;
    loading = true;
    error = "";
    try {
      const result = await modelCatalogRequest(purpose, { ...profile });
      if (
        disposed ||
        request !== generation ||
        endpoint !== profile.endpoint ||
        key !== profile.apiKey
      )
        return;
      models = result;
      manual = false;
      if (!result.length) error = "服务没有返回可用模型，可手动填写模型名称。";
    } catch (cause) {
      if (!disposed && request === generation) {
        error = cause instanceof Error ? cause.message : String(cause);
        manual = true;
      }
    } finally {
      if (!disposed && request === generation) loading = false;
    }
  }
  async function chooseProvider(value: string) {
    clearModels();
    const provider = modelProviders.find((provider) => provider.id === value);
    if (!provider || value === "custom") {
      manual = true;
      customProvider = true;
      return;
    }
    customProvider = false;
    if (!sameEndpoint(profile.endpoint, provider.endpoint))
      profile = {
        ...profile,
        endpoint: provider.endpoint,
        apiKey: "",
        model: "",
        hasApiKey: false,
        dimensions: purpose === "embedding" ? 0 : profile.dimensions,
      };
    lastEndpoint = profile.endpoint;
    await tick();
    autoDetect();
  }
  function endpointInput() {
    if (!sameEndpoint(lastEndpoint, profile.endpoint)) {
      clearModels();
      profile = { ...profile, apiKey: "", hasApiKey: false };
      lastEndpoint = profile.endpoint;
    }
  }
  async function endpointChanged() {
    endpointInput();
    await tick();
    autoDetect();
  }
  async function selectModel(value: string) {
    if (value === "__manual_model__") {
      manual = true;
      await tick();
      modelInput?.focus();
      return;
    }
    const selected = models.find((model) => model.id === value);
    if (purpose === "embedding" && value !== profile.model)
      profile.dimensions = selected?.dimensions ?? 0;
    profile.model = value;
    lastModel = value;
    if (purpose === "embedding" && !profile.dimensions) advancedOpen = true;
  }
  onMount(autoDetect);
  onDestroy(() => {
    disposed = true;
    generation++;
  });
  function modelChanged() {
    if (purpose === "embedding" && lastModel !== profile.model) {
      profile.dimensions =
        models.find((model) => model.id === profile.model)?.dimensions ?? 0;
      advancedOpen = true;
    }
    lastModel = profile.model;
  }
</script>

<section class="model-profile" aria-label={label}>
  <div class="model-section-heading">
    <h4>{label}</h4>
    <span>
      {purpose === "llm" ? "任务摘要与实体提取" : "语义检索与向量索引"}
    </span>
  </div>
  <div class="model-fields">
    <label class="model-field">
      <span>服务提供方</span><select
        aria-label={`${label}服务提供方`}
        value={selectedProvider}
        on:change={(event) => chooseProvider(event.currentTarget.value)}
      >
        {#each modelProviders.filter((provider) => purpose === "llm" || provider.embedding) as provider}<option
            value={provider.id}
          >
            {provider.name}
          </option>{/each}
      </select>
    </label>
    <label class="model-field">
      <span>API 地址</span><input
        aria-label={`${purpose} API 地址`}
        type="url"
        placeholder="https://api.example.com/v1"
        bind:value={profile.endpoint}
        on:input={endpointInput}
        on:change={endpointChanged}
        required={enabled}
        autocomplete="off"
        spellcheck="false"
      />
    </label>
    <div class="model-field">
      <label for={`${purpose}-key`}>API 密钥</label>
      <div class="model-input-action">
        <input
          id={`${purpose}-key`}
          aria-label={`${purpose} API Key`}
          type={showKey ? "text" : "password"}
          bind:value={profile.apiKey}
          on:change={keyChanged}
          placeholder={storedKey
            ? "已保存，留空保留"
            : isLocalEndpoint(profile.endpoint)
              ? "本地服务可留空"
              : "输入供应商 API 密钥"}
          autocomplete="new-password"
        /><button
          type="button"
          class="model-icon"
          aria-label={`${showKey ? "隐藏" : "显示"}${label}密钥`}
          aria-pressed={showKey}
          on:click={() => (showKey = !showKey)}
        >
          {#if showKey}<EyeOff size={16} />{:else}<Eye size={16} />{/if}
        </button>
      </div>
    </div>
    <div class="model-field">
      <label for={`${purpose}-model`}>模型</label>
      <div class="model-input-action">
        {#if visibleModels.length && !manual}<select
            id={`${purpose}-model`}
            aria-label={`${purpose} 模型名称`}
            value={profile.model}
            required={enabled}
            on:change={(event) => selectModel(event.currentTarget.value)}
          >
            <option value="">
              选择模型
            </option>{#if profile.model && !visibleModels.some((model) => model.id === profile.model)}<option
                value={profile.model}
              >
                {profile.model}（当前设置）
              </option>{/if}{#each visibleModels as model}<option
                value={model.id}
              >
                {model.id}
              </option>{/each}<option value="__manual_model__">
              手动输入模型名称…
            </option>
          </select>
        {:else}<input
            id={`${purpose}-model`}
            bind:this={modelInput}
            aria-label={`${purpose} 模型名称`}
            bind:value={profile.model}
            on:change={modelChanged}
            placeholder="选择服务后自动检测，也可手动输入"
            required={enabled}
            spellcheck="false"
            autocomplete="off"
          />{/if}
        <button
          type="button"
          class="model-detect"
          aria-label={`检测${label}列表`}
          disabled={!canDetect || loading}
          on:click={detect}
        >
          {#if loading}<LoaderCircle
              size={14}
              class="model-spinning"
            />{:else}<RefreshCw size={14} />{/if}{loading
            ? "检测中"
            : "检测模型"}
        </button>
      </div>
      {#if error}<p class="model-field-error" role="alert">
          {error}
        </p>{:else if models.length}<p class="model-hint" role="status">
          已检测到 {visibleModels.length} 个可选模型
        </p>{/if}
    </div>
  </div>
  {#if purpose === "embedding"}<details
      class="model-advanced"
      open={advancedOpen || !profile.dimensions}
      on:toggle={(event) => (advancedOpen = event.currentTarget.open)}
    >
      <summary>
        高级设置<span>
          {profile.dimensions ? `${profile.dimensions} 维` : "需要设置向量维度"}
        </span>
      </summary><label class="model-field">
        <span>向量维度</span><input
          aria-label="向量维度"
          type="number"
          min="1"
          max="65536"
          step="1"
          required={enabled}
          bind:value={profile.dimensions}
        />
      </label>
      <p class="model-hint">
        填写该模型实际输出的维度；列表返回维度时会自动填入。
      </p>
    </details>{/if}
  {@render advanced?.()}
</section>
