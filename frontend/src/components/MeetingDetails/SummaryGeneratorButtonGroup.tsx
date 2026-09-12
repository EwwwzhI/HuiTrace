"use client";

import { SummaryTemplateEditor } from './SummaryTemplateEditor';
import { ModelConfig, ModelSettingsModal } from '@/components/ModelSettingsModal';
import {
  Dialog,
  DialogContent,
  DialogTrigger,
  DialogTitle,
} from "@/components/ui/dialog"
import { VisuallyHidden } from "@/components/ui/visually-hidden"
import { Button } from '@/components/ui/button';
import { ButtonGroup } from '@/components/ui/button-group';
import { Sparkles, Settings, Loader2, Square } from 'lucide-react';
import Analytics from '@/lib/analytics';
import { invoke } from '@tauri-apps/api/core';
import { openExternalUrl } from '@/services/systemService';
import { getOllamaModels } from '@/services/providerModelsService';
import { toast } from 'sonner';
import { useState, useEffect, ReactNode } from 'react';
import { isOllamaNotInstalledError } from '@/lib/utils';
import { BuiltInModelInfo } from '@/lib/builtin-ai';
import { translateUI } from '@/i18n';
import { useUiTranslation } from '@/i18n/client';



interface SummaryGeneratorButtonGroupProps {
  languageSlot?: ReactNode;
  modelConfig: ModelConfig;
  setModelConfig: (config: ModelConfig | ((prev: ModelConfig) => ModelConfig)) => void;
  onSaveModelConfig: (config?: ModelConfig) => Promise<void>;
  onGenerateSummary: (customPrompt: string) => Promise<void>;
  onStopGeneration: () => void;
  customPrompt: string;
  summaryStatus: 'idle' | 'processing' | 'summarizing' | 'regenerating' | 'completed' | 'error';
  availableTemplates: Array<{ id: string, name: string, description: string, source?: 'builtin' | 'bundled' | 'custom' }>;
  selectedTemplate: string;
  onTemplateSelect: (templateId: string, templateName: string) => void;
  hasTranscripts?: boolean;
  hasSummary?: boolean;
  isModelConfigLoading?: boolean;
  onOpenModelSettings?: (openFn: () => void) => void;
}

export function SummaryGeneratorButtonGroup({
  modelConfig,
  setModelConfig,
  onSaveModelConfig,
  onGenerateSummary,
  onStopGeneration,
  customPrompt,
  summaryStatus,
  availableTemplates,
  selectedTemplate,
  onTemplateSelect,
  hasTranscripts = true,
  hasSummary = false,
  isModelConfigLoading = false,
  onOpenModelSettings,
  languageSlot
}: SummaryGeneratorButtonGroupProps) {
  useUiTranslation();
  const [isCheckingModels, setIsCheckingModels] = useState(false);
  const [settingsDialogOpen, setSettingsDialogOpen] = useState(false);

  // Expose the function to open the modal via callback registration
  useEffect(() => {
    if (onOpenModelSettings) {
      // Register our open dialog function with the parent by calling the callback
      // This allows the parent to store a reference to this function
      const openDialog = () => {
        console.log('📱 Opening model settings dialog via callback');
        setSettingsDialogOpen(true);
      };

      // Call the parent's callback with our open function
      // Note: This assumes onOpenModelSettings accepts a function parameter
      // We'll need to adjust the signature
      onOpenModelSettings(openDialog);
    }
  }, [onOpenModelSettings]);

  if (!hasTranscripts) {
    return null;
  }

  const checkBuiltInAIModelsAndGenerate = async () => {
    setIsCheckingModels(true);
    try {
      const selectedModel = modelConfig.model;

      // Check if specific model is configured
      if (!selectedModel) {
        toast.error(translateUI("No built-in AI model selected"), {
          get description() { return translateUI("Please select a model in settings"); },
          duration: 5000,
        });
        setSettingsDialogOpen(true);
        return;
      }

      // Check model readiness (with filesystem refresh)
      const isReady = await invoke<boolean>('builtin_ai_is_model_ready', {
        modelName: selectedModel,
        refresh: true,
      });

      if (isReady) {
        // Model is available, proceed with generation
        onGenerateSummary(customPrompt);
        return;
      }

      // Model not ready - check detailed status
      const modelInfo = await invoke<BuiltInModelInfo | null>('builtin_ai_get_model_info', {
        modelName: selectedModel,
      });

      if (!modelInfo) {
        toast.error(translateUI("Model not found"), {
          description: `Could not find information for model: ${selectedModel}`,
          duration: 5000,
        });
        setSettingsDialogOpen(true);
        return;
      }

      // Handle different model states
      const status = modelInfo.status;

      if (status.type === 'downloading') {
        toast.info(translateUI("Model download in progress"), {
          description: `${selectedModel} is downloading (${status.progress}%). Please wait until download completes.`,
          duration: 5000,
        });
        return;
      }

      if (status.type === 'not_downloaded') {
        toast.error(translateUI("Model not downloaded"), {
          description: `${selectedModel} needs to be downloaded before use. Opening model settings...`,
          duration: 5000,
        });
        setSettingsDialogOpen(true);
        return;
      }

      if (status.type === 'corrupted') {
        toast.error(translateUI("Model file corrupted"), {
          description: `${selectedModel} file is corrupted. Please delete and re-download.`,
          duration: 7000,
        });
        setSettingsDialogOpen(true);
        return;
      }

      if (status.type === 'error') {
        toast.error(translateUI("Model error"), {
          description: status.Error || translateUI("An error occurred with the model"),
          duration: 5000,
        });
        setSettingsDialogOpen(true);
        return;
      }

      // Fallback
      toast.error(translateUI("Model not available"), {
        get description() { return translateUI("The selected model is not ready for use"); },
        duration: 5000,
      });
      setSettingsDialogOpen(true);

    } catch (error) {
      console.error('Error checking built-in AI models:', error);
      toast.error(translateUI("Failed to check model status"), {
        description: error instanceof Error ? error.message : String(error),
        duration: 5000,
      });
    } finally {
      setIsCheckingModels(false);
    }
  };

  const checkOllamaModelsAndGenerate = async () => {
    // Handle built-in AI provider
    if (modelConfig.provider === 'builtin-ai') {
      await checkBuiltInAIModelsAndGenerate();
      return;
    }

    // Only check for Ollama provider
    if (modelConfig.provider !== 'ollama') {
      onGenerateSummary(customPrompt);
      return;
    }

    setIsCheckingModels(true);
    try {
      const endpoint = modelConfig.ollamaEndpoint || null;
      const models = await getOllamaModels(endpoint);

      if (!models || models.length === 0) {
        // No models available, show message and open settings
        toast.error(
          translateUI("No Ollama models found. Please download gemma2:2b from Model Settings."),
          { duration: 5000 }
        );
        setSettingsDialogOpen(true);
        return;
      }

      // Models are available, proceed with generation
      onGenerateSummary(customPrompt);
    } catch (error) {
      console.error('Error checking Ollama models:', error);
      const errorMessage = error instanceof Error ? error.message : String(error);

      if (isOllamaNotInstalledError(errorMessage)) {
        // Ollama is not installed - show specific message with download link
        toast.error(
          translateUI("Ollama is not installed"),
          {
            get description() { return translateUI("Please download and install Ollama to use local models."); },
            duration: 7000,
            action: {
              get label() { return translateUI("Download"); },
              onClick: () => openExternalUrl('https://ollama.com/download')
            }
          }
        );
      } else {
        // Other error - generic message
        toast.error(
          translateUI("Failed to check Ollama models. Please check if Ollama is running and download a model."),
          { duration: 5000 }
        );
      }
      setSettingsDialogOpen(true);
    } finally {
      setIsCheckingModels(false);
    }
  };

  const isGenerating = summaryStatus === 'processing' || summaryStatus === 'summarizing' || summaryStatus === 'regenerating';

  // Responsive labels — the summary panel is CAPPED at 640px (page-content),
  // so viewport breakpoints must run later than the app-wide `lg` convention:
  //  - < xl: everything icon-only (panel can be as narrow as ~340px);
  //  - xl+:  the primary Generate/Stop label appears;
  //  - 2xl+: language value + Save/Copy labels appear (~620px total, fits 640).
  // "AI Model"/"Template" stay icon-only at EVERY width: all six labels total
  // ~730px and can never fit the 640px cap — these two are static names whose
  // meaning the icons + title tooltips already carry.
  return (
    <ButtonGroup className="shrink-0">
      {/* Generate Summary or Stop button */}
      {isGenerating ? (
        <Button
          variant="outline"
          size="sm"
          className="shrink-0 border-destructive/30 text-destructive hover:bg-destructive/10 dark:border-destructive/30 dark:text-destructive dark:hover:bg-destructive/10 xl:px-4"
          onClick={() => {
            Analytics.trackButtonClick('stop_summary_generation', 'meeting_details');
            onStopGeneration();
          }}
          title={translateUI("Stop summary generation")}
        >
          <Square className="xl:mr-2" size={18} fill="currentColor" />
          <span className="hidden xl:inline">{translateUI("Stop")}</span>
        </Button>
      ) : (
        <Button
          size="sm"
          className="shrink-0 bg-primary text-primary-foreground shadow-sm hover:bg-primary/90 xl:px-4"
          onClick={() => {
            Analytics.trackButtonClick('generate_summary', 'meeting_details');
            checkOllamaModelsAndGenerate();
          }}
          disabled={isCheckingModels || isModelConfigLoading}
          title={
            isModelConfigLoading
              ? translateUI("Loading model configuration...")
              : isCheckingModels
                ? translateUI("Checking models...")
                : hasSummary ? translateUI("Regenerate AI Summary") : translateUI("Generate AI Summary")
          }
        >
          {isCheckingModels || isModelConfigLoading ? (
            <>
              <Loader2 className="animate-spin xl:mr-2" size={18} />
              <span className="hidden xl:inline">{translateUI("Processing...")}</span>
            </>
          ) : (
            <>
              <Sparkles className="xl:mr-2" size={18} />
              <span className="hidden xl:inline">{hasSummary ? translateUI("Regenerate Summary") : translateUI("Generate Summary")}</span>
            </>
          )}
        </Button>
      )}

      {languageSlot}

      {/* Settings button — icon-only at every width (see width budget above);
          the title tooltip names it. */}
      <Dialog open={settingsDialogOpen} onOpenChange={setSettingsDialogOpen}>
        <DialogTrigger asChild>
          <Button
            variant="outline"
            size="sm"
            className="shrink-0"
            title={translateUI("Summary Settings (AI Model)")}
            aria-label={translateUI("Summary settings (AI model)")}
          >
            <Settings />
          </Button>
        </DialogTrigger>
        <DialogContent
          aria-describedby={undefined}
        >
          <VisuallyHidden>
            <DialogTitle>{translateUI("Model Settings")}</DialogTitle>
          </VisuallyHidden>
          <ModelSettingsModal
            onSave={async (config) => {
              await onSaveModelConfig(config);
              setSettingsDialogOpen(false);
            }}
            modelConfig={modelConfig}
            setModelConfig={setModelConfig}
            skipInitialFetch={true}
            layout="dialog"
          />
        </DialogContent>
      </Dialog>

      <SummaryTemplateEditor templates={availableTemplates} selected={selectedTemplate} onSelect={onTemplateSelect} disabled={isGenerating} />
    </ButtonGroup>
  );
}
